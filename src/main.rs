//! QR Kod Sistemi — Rust (Axum) + MySQL/MariaDB
//! - Giriş + roller (admin / normal kullanıcı). Sadece admin kullanıcı yönetir.
//! - Sadece kayıtlı kodlar için QR (tam URL kodlar -> telefon kamerasıyla açılır)
//! - Kodlara detaylı alanlar: metin / sayı / fotoğraf (jpg,png,webp)
//! - Tarama: link ise yönlendirir, değilse detay sayfasını açar (herkese açık)

use argon2::password_hash::{rand_core::{OsRng, RngCore}, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use qrcode::{render::svg, QrCode};
use serde::{Deserialize, Serialize};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    pool: MySqlPool,
}

#[derive(Serialize, sqlx::FromRow)]
struct CodeRecord {
    id: i64,
    code: String,
    payload: Option<String>,
    created_at: chrono::NaiveDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct FieldMeta {
    id: i64,
    title: String,
    field_type: String,
    value_text: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct UserRecord {
    id: i64,
    username: String,
    is_admin: i8,
    must_change_password: i8,
    created_at: chrono::NaiveDateTime,
}

#[derive(Deserialize)]
struct CodeInput {
    code: String,
    payload: Option<String>,
}

#[derive(Deserialize)]
struct LoginInput {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct NewUser {
    username: String,
    password: String,
    #[serde(default)]
    is_admin: bool,
}

#[derive(Deserialize)]
struct ResetPw {
    password: String,
}

#[derive(Deserialize)]
struct ChangePw {
    current_password: String,
    new_password: String,
}

#[tokio::main]
async fn main() {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://root:password@localhost:3306/qrdb".to_string());

    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("Veritabanına bağlanılamadı. DATABASE_URL doğru mu?");

    // --- Tablolar ---
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS codes (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            code VARCHAR(255) NOT NULL UNIQUE,
            payload TEXT NULL,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )"#,
    ).execute(&pool).await.expect("codes tablosu");

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS code_fields (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            code_id BIGINT NOT NULL,
            title VARCHAR(255) NOT NULL,
            field_type VARCHAR(20) NOT NULL,
            value_text TEXT NULL,
            image_data LONGBLOB NULL,
            image_mime VARCHAR(100) NULL,
            sort_order INT NOT NULL DEFAULT 0,
            FOREIGN KEY (code_id) REFERENCES codes(id) ON DELETE CASCADE
        )"#,
    ).execute(&pool).await.expect("code_fields tablosu");

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS users (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            username VARCHAR(100) NOT NULL UNIQUE,
            password_hash VARCHAR(255) NOT NULL,
            is_admin TINYINT NOT NULL DEFAULT 0,
            must_change_password TINYINT NOT NULL DEFAULT 0,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        )"#,
    ).execute(&pool).await.expect("users tablosu");

    // Eski kurulumlar için eksik kolonları ekle (MariaDB IF NOT EXISTS destekler)
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS must_change_password TINYINT NOT NULL DEFAULT 0").execute(&pool).await;
    let _ = sqlx::query("ALTER TABLE users ADD COLUMN IF NOT EXISTS is_admin TINYINT NOT NULL DEFAULT 0").execute(&pool).await;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS sessions (
            token VARCHAR(64) PRIMARY KEY,
            user_id BIGINT NOT NULL,
            expires_at DATETIME NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )"#,
    ).execute(&pool).await.expect("sessions tablosu");

    seed_admin(&pool).await;

    // En az bir admin garanti et (eski kurulumda ilk kullanıcıyı admin yap)
    let admin_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE is_admin = 1")
        .fetch_one(&pool).await.unwrap_or((0,));
    if admin_count.0 == 0 {
        let _ = sqlx::query("UPDATE users SET is_admin = 1 ORDER BY id LIMIT 1").execute(&pool).await;
    }

    let state = AppState { pool };

    let app = Router::new()
        .route("/", get(admin_page))
        .route("/login", get(login_page))
        .route("/change-password", get(change_password_page))
        .route("/scan", get(scan_page))
        .route("/c/:code", get(detail_page))
        .route("/qr/:code", get(qr_svg))
        .route("/api/me", get(me))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/change-password", post(change_password))
        .route("/api/lookup/:code", get(lookup))
        .route("/api/field-image/:fid", get(field_image))
        .route("/api/codes", get(list_codes).post(create_code))
        .route("/api/codes/:id", put(update_code).delete(delete_code))
        .route("/api/codes/:id/fields", get(list_fields).post(add_field))
        .route("/api/fields/:fid", delete(delete_field))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:id", delete(delete_user))
        .route("/api/users/:id/reset", post(reset_user_password))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 10MB yükleme
        .nest_service("/static", ServeDir::new("static"))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));

    let config = RustlsConfig::from_pem_file(
        PathBuf::from("certs/cert.pem"),
        PathBuf::from("certs/key.pem"),
    )
    .await
    .expect("Sertifika yüklenemedi. certs/cert.pem ve certs/key.pem var mı?");

    println!("Sunucu calisiyor: https://localhost:3000");
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// ---------- Şifre / oturum yardımcıları ----------

fn hash_password(pw: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default().hash_password(pw.as_bytes(), &salt).expect("hash").to_string()
}
fn verify_password(pw: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(p) => Argon2::default().verify_password(pw.as_bytes(), &p).is_ok(),
        Err(_) => false,
    }
}
fn new_token() -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

async fn seed_admin(pool: &MySqlPool) {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users").fetch_one(pool).await.expect("user say");
    if count.0 == 0 {
        let user = std::env::var("ADMIN_USER").unwrap_or_else(|_| "admin".into());
        let pass = std::env::var("ADMIN_PASS").unwrap_or_else(|_| "admin123".into());
        let hash = hash_password(&pass);
        sqlx::query("INSERT INTO users (username, password_hash, is_admin, must_change_password) VALUES (?, ?, 1, 0)")
            .bind(&user).bind(&hash).execute(pool).await.expect("admin ekle");
        println!("İlk admin olusturuldu -> kullanici: {user} / sifre: {pass}");
        if pass == "admin123" {
            println!("!! UYARI: varsayilan sifre kullaniliyor. ADMIN_PASS ile degistir.");
        }
    }
}

/// Çerez token'ından kullanıcı id (yoksa None).
async fn current_user(headers: &HeaderMap, pool: &MySqlPool) -> Option<i64> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let token = cookie.split(';').map(|s| s.trim()).find_map(|s| s.strip_prefix("session="))?;
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM sessions WHERE token = ? AND expires_at > NOW()")
            .bind(token).fetch_optional(pool).await.ok()?;
    row.map(|r| r.0)
}

async fn is_admin(pool: &MySqlPool, uid: i64) -> bool {
    let row: Option<(i8,)> = sqlx::query_as("SELECT is_admin FROM users WHERE id = ?")
        .bind(uid).fetch_optional(pool).await.ok().flatten();
    row.map(|r| r.0 != 0).unwrap_or(false)
}

/// Giriş kontrolü: giriş yoksa 401 döner.
macro_rules! require_login {
    ($headers:expr, $pool:expr) => {
        match current_user(&$headers, &$pool).await {
            Some(uid) => uid,
            None => return (StatusCode::UNAUTHORIZED, "Giris gerekli").into_response(),
        }
    };
}

// ---------- Sayfalar ----------

async fn admin_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(uid) = current_user(&headers, &state.pool).await else {
        return Redirect::to("/login").into_response();
    };
    let mc: Option<(i8,)> = sqlx::query_as("SELECT must_change_password FROM users WHERE id = ?")
        .bind(uid).fetch_optional(&state.pool).await.ok().flatten();
    if mc.map(|r| r.0).unwrap_or(0) != 0 {
        return Redirect::to("/change-password").into_response();
    }
    Html(include_str!("../static/admin.html")).into_response()
}

async fn login_page() -> Html<&'static str> { Html(include_str!("../static/login.html")) }
async fn scan_page() -> Html<&'static str> { Html(include_str!("../static/scan.html")) }
async fn detail_page() -> Html<&'static str> { Html(include_str!("../static/detail.html")) }

async fn change_password_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&headers, &state.pool).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    Html(include_str!("../static/change-password.html")).into_response()
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let uid = require_login!(headers, state.pool);
    let row: Option<(String, i8)> = sqlx::query_as("SELECT username, is_admin FROM users WHERE id = ?")
        .bind(uid).fetch_optional(&state.pool).await.ok().flatten();
    match row {
        Some((u, a)) => Json(serde_json::json!({"username": u, "is_admin": a != 0})).into_response(),
        None => (StatusCode::UNAUTHORIZED, "Giris gerekli").into_response(),
    }
}

// ---------- Giriş / çıkış / şifre ----------

async fn login(State(state): State<AppState>, Json(input): Json<LoginInput>) -> Response {
    let row: Option<(i64, String, i8)> = sqlx::query_as(
        "SELECT id, password_hash, must_change_password FROM users WHERE username = ?",
    ).bind(&input.username).fetch_optional(&state.pool).await.ok().flatten();

    let Some((uid, hash, must_change)) = row else {
        return (StatusCode::UNAUTHORIZED, "Kullanici adi veya sifre hatali").into_response();
    };
    if !verify_password(&input.password, &hash) {
        return (StatusCode::UNAUTHORIZED, "Kullanici adi veya sifre hatali").into_response();
    }
    let token = new_token();
    let _ = sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, DATE_ADD(NOW(), INTERVAL 7 DAY))")
        .bind(&token).bind(uid).execute(&state.pool).await;
    let cookie = format!("session={token}; HttpOnly; Path=/; Max-Age=604800; SameSite=Lax");
    ([(header::SET_COOKIE, cookie)], Json(serde_json::json!({"ok": true, "must_change": must_change != 0}))).into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
        if let Some(token) = cookie.split(';').map(|s| s.trim()).find_map(|s| s.strip_prefix("session=")) {
            let _ = sqlx::query("DELETE FROM sessions WHERE token = ?").bind(token).execute(&state.pool).await;
        }
    }
    ([(header::SET_COOKIE, "session=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax")], Json(serde_json::json!({"ok": true}))).into_response()
}

async fn change_password(State(state): State<AppState>, headers: HeaderMap, Json(input): Json<ChangePw>) -> Response {
    let uid = require_login!(headers, state.pool);
    if input.new_password.len() < 4 {
        return (StatusCode::BAD_REQUEST, "Yeni sifre en az 4 karakter olmali").into_response();
    }
    let row: Option<(String,)> = sqlx::query_as("SELECT password_hash FROM users WHERE id = ?")
        .bind(uid).fetch_optional(&state.pool).await.ok().flatten();
    let Some((hash,)) = row else { return (StatusCode::INTERNAL_SERVER_ERROR, "Kullanici yok").into_response(); };
    if !verify_password(&input.current_password, &hash) {
        return (StatusCode::UNAUTHORIZED, "Mevcut sifre hatali").into_response();
    }
    let new_hash = hash_password(&input.new_password);
    let _ = sqlx::query("UPDATE users SET password_hash = ?, must_change_password = 0 WHERE id = ?")
        .bind(&new_hash).bind(uid).execute(&state.pool).await;
    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------- QR (giriş + kayıtlı kod; tam URL kodlar) ----------

async fn qr_svg(State(state): State<AppState>, headers: HeaderMap, Path(code): Path<String>) -> Response {
    if current_user(&headers, &state.pool).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "Giris gerekli").into_response();
    }
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM codes WHERE code = ?")
        .bind(&code).fetch_optional(&state.pool).await.ok().flatten();
    if exists.is_none() {
        return (StatusCode::NOT_FOUND, "Bu kod kayitli degil, QR uretilemez").into_response();
    }
    // BASE_URL verilmişse tam URL kodla (telefon kamerasıyla açılsın)
    let content = match std::env::var("BASE_URL") {
        Ok(b) if !b.trim().is_empty() => format!("{}/c/{}", b.trim_end_matches('/'), code),
        _ => code.clone(),
    };
    match QrCode::new(content.as_bytes()) {
        Ok(qr) => {
            let svg = qr.render::<svg::Color>().min_dimensions(240, 240).quiet_zone(true).build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
        }
        Err(_) => (StatusCode::BAD_REQUEST, "Gecersiz kod").into_response(),
    }
}

// ---------- Tarama / detay (açık) ----------

async fn lookup(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    let rec: Option<CodeRecord> = sqlx::query_as(
        "SELECT id, code, payload, created_at FROM codes WHERE code = ?",
    ).bind(&code).fetch_optional(&state.pool).await.ok().flatten();

    let Some(rec) = rec else {
        return (StatusCode::NOT_FOUND, "Kod bulunamadi").into_response();
    };
    let fields: Vec<FieldMeta> = sqlx::query_as(
        "SELECT id, title, field_type, value_text FROM code_fields WHERE code_id = ? ORDER BY sort_order, id",
    ).bind(rec.id).fetch_all(&state.pool).await.unwrap_or_default();

    let fields_json: Vec<serde_json::Value> = fields.iter().map(|f| {
        if f.field_type == "image" {
            serde_json::json!({"id": f.id, "title": f.title, "field_type": "image", "image_url": format!("/api/field-image/{}", f.id)})
        } else {
            serde_json::json!({"id": f.id, "title": f.title, "field_type": f.field_type, "value_text": f.value_text})
        }
    }).collect();

    Json(serde_json::json!({
        "code": rec.code, "payload": rec.payload, "created_at": rec.created_at, "fields": fields_json
    })).into_response()
}

async fn field_image(State(state): State<AppState>, Path(fid): Path<i64>) -> Response {
    let row: Option<(Option<Vec<u8>>, Option<String>)> =
        sqlx::query_as("SELECT image_data, image_mime FROM code_fields WHERE id = ?")
            .bind(fid).fetch_optional(&state.pool).await.ok().flatten();
    match row {
        Some((Some(data), Some(mime))) => ([(header::CONTENT_TYPE, mime)], data).into_response(),
        _ => (StatusCode::NOT_FOUND, "Resim yok").into_response(),
    }
}

// ---------- Kod yönetimi (giriş gerekli) ----------

async fn list_codes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let _ = require_login!(headers, state.pool);
    match sqlx::query_as::<_, CodeRecord>("SELECT id, code, payload, created_at FROM codes ORDER BY id DESC")
        .fetch_all(&state.pool).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_code(State(state): State<AppState>, headers: HeaderMap, Json(input): Json<CodeInput>) -> Response {
    let _ = require_login!(headers, state.pool);
    match sqlx::query("INSERT INTO codes (code, payload) VALUES (?, ?)")
        .bind(&input.code).bind(&input.payload).execute(&state.pool).await {
        Ok(r) => Json(serde_json::json!({"ok": true, "id": r.last_insert_id()})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn update_code(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>, Json(input): Json<CodeInput>) -> Response {
    let _ = require_login!(headers, state.pool);
    match sqlx::query("UPDATE codes SET code = ?, payload = ? WHERE id = ?")
        .bind(&input.code).bind(&input.payload).bind(id).execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn delete_code(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let _ = require_login!(headers, state.pool);
    match sqlx::query("DELETE FROM codes WHERE id = ?").bind(id).execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

// ---------- Alanlar (giriş gerekli) ----------

async fn list_fields(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let _ = require_login!(headers, state.pool);
    let fields: Vec<FieldMeta> = sqlx::query_as(
        "SELECT id, title, field_type, value_text FROM code_fields WHERE code_id = ? ORDER BY sort_order, id",
    ).bind(id).fetch_all(&state.pool).await.unwrap_or_default();
    let out: Vec<serde_json::Value> = fields.iter().map(|f| {
        let mut v = serde_json::json!({"id": f.id, "title": f.title, "field_type": f.field_type, "value_text": f.value_text});
        if f.field_type == "image" {
            v["image_url"] = serde_json::json!(format!("/api/field-image/{}", f.id));
        }
        v
    }).collect();
    Json(out).into_response()
}

async fn add_field(State(state): State<AppState>, headers: HeaderMap, Path(code_id): Path<i64>, mut mp: Multipart) -> Response {
    let _ = require_login!(headers, state.pool);

    let mut title = String::new();
    let mut ftype = String::new();
    let mut vtext: Option<String> = None;
    let mut img: Option<Vec<u8>> = None;
    let mut mime: Option<String> = None;

    while let Ok(Some(field)) = mp.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "title" => title = field.text().await.unwrap_or_default(),
            "field_type" => ftype = field.text().await.unwrap_or_default(),
            "value_text" => vtext = field.text().await.ok(),
            "file" => {
                mime = field.content_type().map(|s| s.to_string());
                img = field.bytes().await.ok().map(|b| b.to_vec());
            }
            _ => {}
        }
    }

    if title.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Baslik gerekli").into_response();
    }

    let so: (i64,) = sqlx::query_as("SELECT CAST(COALESCE(MAX(sort_order)+1, 0) AS SIGNED) FROM code_fields WHERE code_id = ?")
        .bind(code_id).fetch_one(&state.pool).await.unwrap_or((0,));

    let res = match ftype.as_str() {
        "text" => {
            sqlx::query("INSERT INTO code_fields (code_id, title, field_type, value_text, sort_order) VALUES (?, ?, 'text', ?, ?)")
                .bind(code_id).bind(title.trim()).bind(&vtext).bind(so.0).execute(&state.pool).await
        }
        "number" => {
            if let Some(v) = &vtext {
                if !v.trim().is_empty() && v.trim().parse::<f64>().is_err() {
                    return (StatusCode::BAD_REQUEST, "Gecersiz sayi").into_response();
                }
            }
            sqlx::query("INSERT INTO code_fields (code_id, title, field_type, value_text, sort_order) VALUES (?, ?, 'number', ?, ?)")
                .bind(code_id).bind(title.trim()).bind(&vtext).bind(so.0).execute(&state.pool).await
        }
        "image" => {
            let m = mime.clone().unwrap_or_default();
            let ok = matches!(m.as_str(), "image/jpeg" | "image/jpg" | "image/png" | "image/webp");
            if !ok {
                return (StatusCode::BAD_REQUEST, "Sadece jpg, png veya webp").into_response();
            }
            let data = img.unwrap_or_default();
            if data.is_empty() {
                return (StatusCode::BAD_REQUEST, "Dosya bos").into_response();
            }
            sqlx::query("INSERT INTO code_fields (code_id, title, field_type, image_data, image_mime, sort_order) VALUES (?, ?, 'image', ?, ?, ?)")
                .bind(code_id).bind(title.trim()).bind(&data).bind(&m).bind(so.0).execute(&state.pool).await
        }
        _ => return (StatusCode::BAD_REQUEST, "Gecersiz tip").into_response(),
    };

    match res {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn delete_field(State(state): State<AppState>, headers: HeaderMap, Path(fid): Path<i64>) -> Response {
    let _ = require_login!(headers, state.pool);
    match sqlx::query("DELETE FROM code_fields WHERE id = ?").bind(fid).execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

// ---------- Kullanıcı yönetimi (SADECE ADMIN) ----------

async fn list_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let uid = require_login!(headers, state.pool);
    if !is_admin(&state.pool, uid).await {
        return (StatusCode::FORBIDDEN, "Bu islem icin yonetici olmalisin").into_response();
    }
    match sqlx::query_as::<_, UserRecord>(
        "SELECT id, username, is_admin, must_change_password, created_at FROM users ORDER BY id",
    ).fetch_all(&state.pool).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn create_user(State(state): State<AppState>, headers: HeaderMap, Json(input): Json<NewUser>) -> Response {
    let uid = require_login!(headers, state.pool);
    if !is_admin(&state.pool, uid).await {
        return (StatusCode::FORBIDDEN, "Bu islem icin yonetici olmalisin").into_response();
    }
    if input.username.trim().is_empty() || input.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, "Kullanici adi bos olamaz, sifre en az 4 karakter").into_response();
    }
    let hash = hash_password(&input.password);
    let admin_flag: i8 = if input.is_admin { 1 } else { 0 };
    match sqlx::query("INSERT INTO users (username, password_hash, is_admin, must_change_password) VALUES (?, ?, ?, 1)")
        .bind(input.username.trim()).bind(&hash).bind(admin_flag).execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn reset_user_password(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>, Json(input): Json<ResetPw>) -> Response {
    let uid = require_login!(headers, state.pool);
    if !is_admin(&state.pool, uid).await {
        return (StatusCode::FORBIDDEN, "Bu islem icin yonetici olmalisin").into_response();
    }
    if input.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, "Gecici sifre en az 4 karakter").into_response();
    }
    let hash = hash_password(&input.password);
    let _ = sqlx::query("UPDATE users SET password_hash = ?, must_change_password = 1 WHERE id = ?")
        .bind(&hash).bind(id).execute(&state.pool).await;
    let _ = sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&state.pool).await;
    Json(serde_json::json!({"ok": true})).into_response()
}

async fn delete_user(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let uid = require_login!(headers, state.pool);
    if !is_admin(&state.pool, uid).await {
        return (StatusCode::FORBIDDEN, "Bu islem icin yonetici olmalisin").into_response();
    }
    if uid == id {
        return (StatusCode::BAD_REQUEST, "Kendi hesabini silemezsin").into_response();
    }
    // Son admini silmeyi engelle
    let target_admin = is_admin(&state.pool, id).await;
    if target_admin {
        let admins: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE is_admin = 1")
            .fetch_one(&state.pool).await.unwrap_or((0,));
        if admins.0 <= 1 {
            return (StatusCode::BAD_REQUEST, "Son yoneticiyi silemezsin").into_response();
        }
    }
    match sqlx::query("DELETE FROM users WHERE id = ?").bind(id).execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}
