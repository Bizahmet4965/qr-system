# Kurulum

```bash
# 1. Projeyi çek
git clone https://github.com/Bizahmet4965/qr-system.git
cd qr-system

# 2. Kurulum scriptini çalıştır — gerisini o halleder
bash setup.sh
```

`setup.sh` otomatik olarak şunları yapar:
- Rust yoksa kurar (`rustup`)
- Eksik sistem paketlerini kurar (`mkcert`, `nss`, `libssl-dev` vb.)
- MariaDB servisini başlatır
- Seni soru soru veritabanı/admin/BASE_URL için yönlendirir
- `.env` oluşturur
- Veritabanı + kullanıcı oluşturur (`sudo mysql` ile)
- TLS sertifikasını üretir (`certs/cert.pem`, `certs/key.pem`)
- Projeyi derler (`cargo build --release`)
- İsteğe bağlı: systemd servisi kurar (açılışta otomatik başlasın)

### Kurulum sonrası manuel çalıştırmak için
```bash
cd qr-system
export $(cat .env | xargs)
./target/release/qr-system
# veya: cargo run --release
```
---

## 4. Notlar

| Şifre sıfırlama | `DELETE FROM sessions; DELETE FROM users;` → yeniden başlat → yeni admin seed'lenir |
