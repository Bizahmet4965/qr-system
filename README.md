# QR Kod Sistemi (Rust + MySQL)

Veritabanındaki bir **kod**u QR'a çevirir; kamerası olan **her cihazdan**
(telefon / PC / tablet) tarayıp veritabanından sorgulamayı sağlayan tek bir web uygulaması.

## Mimari (özet)

```
[ MySQL: codes tablosu ]  <-->  [ Rust / Axum sunucu ]  <-->  [ Web sayfası (her tarayıcı) ]
                                       |  GET  /qr/:code      -> kodu QR (SVG) görseli yapar
                                       |  GET  /api/lookup/:code -> taranan kodu DB'de arar
                                       |  POST /api/codes      -> yeni kod ekler
                                       |  GET  /               -> web arayüzü
```

- **Kamera ile tarama** tamamen tarayıcıda olur (`html5-qrcode`). Bu yüzden
  telefon, PC, tablet ayrımı yoktur; kamerası + tarayıcısı olan her cihaz çalışır.
- Tarayıcı kod okur -> sunucuya gönderir -> sunucu veritabanından eşleşeni döner.

## Kurulum

1. **Rust** kurulu olsun (https://rustup.rs).
2. **MySQL** çalışıyor olsun. Veritabanını kur:
   ```bash
   mysql -u root -p < schema.sql
   ```
3. Bağlantı dizesini ortam değişkenine ver:
   ```bash
   export DATABASE_URL="mysql://root:SIFREN@localhost:3306/qrdb"
   ```
   (Windows PowerShell: `$env:DATABASE_URL="mysql://root:SIFREN@localhost:3306/qrdb"`)
4. Çalıştır:
   ```bash
   cargo run
   ```
5. Tarayıcıdan aç: **http://localhost:3000**

## Kullanım

- **Kod Ekle** sekmesi: veritabanına yeni bir kod + içerik kaydet.
- **QR Göster** sekmesi: bir kodun QR görselini üret (yazdır/paylaş).
- **Tara** sekmesi: kamerayla QR oku; kod veritabanında aranır, içeriği gösterilir.

> Not: Tarayıcılar kamerayı yalnızca **HTTPS** veya **localhost** üzerinde açar.
> Başka cihazlardan (telefon) test ederken sunucuyu HTTPS arkasına alman gerekir
> (örn. Caddy / nginx ters proxy ile, ya da `mkcert` ile yerel sertifika).

## MSSQL kullanmak istersen

`sqlx` (bu projedeki sürücü) **MSSQL desteklemez**. MSSQL için Rust'ta
[`tiberius`](https://crates.io/crates/tiberius) sürücüsü kullanılır. O durumda
`Cargo.toml`'daki `sqlx` satırını çıkarıp `tiberius` + `tokio-util` eklemen ve
`main.rs` içindeki sorguları tiberius API'siyle yazman gerekir. Sorgu mantığı aynı,
sadece bağlantı/sorgu çağrıları değişir. İstersen MSSQL sürümünü de hazırlayabilirim.

## Üretim için sonraki adımlar
- Kimlik doğrulama (kod ekleme/silme yetkisi).
- Kod oluştururken benzersiz/şifreli token üretimi (tahmin edilemesin diye).
- HTTPS (mutlaka, mobil kamera için şart).
- Rate-limit ve loglama.
