# GitHub'a Yükleme ve Başka Bilgisayara Kurulum

## 1. GitHub'a Yükleme (Bir Kez)

### Repo oluştur
1. https://github.com/new adresine git.
2. Repository name: `qr-system`
3. **Private** seç (önerilen — veritabanı şema bilgileri var).
4. **"Add a README"** kutusunu işaretleme (zaten kendi README'miz var).
5. **Create repository** tıkla.

### Yerel klasörü repoya bağla ve at
```bash
cd /home/ahmet/okulproje        # mevcut proje klasörün

git init
git add .
git commit -m "ilk commit"

# GitHub'ın gösterdiği URL'i kullan (aşağıdaki örnek):
git remote add origin https://github.com/KULLANICI_ADIN/qr-system.git
git branch -M main
git push -u origin main
```

> `.env` ve `certs/` klasörü `.gitignore`'da olduğu için repoya **gitmez**.
> Şifreler ve sertifikalar güvende kalır.

---

## 2. Başka Bir Bilgisayara Kurulum (Sıfırdan)

```bash
# 1. Projeyi çek
git clone https://github.com/KULLANICI_ADIN/qr-system.git
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

## 3. Güncelleme (Yeni Kod Gelince)

```bash
cd qr-system
git pull
cargo build --release

# systemd servisi varsa yeniden başlat:
sudo systemctl restart qr-system
```

---

## 4. Notlar

| Konu | Detay |
|------|-------|
| `.env` | Repoda yok, her makinede ayrı oluşturulur (`setup.sh` yapar) |
| `certs/` | Repoda yok, her makinede `mkcert` ile üretilir (`setup.sh` yapar) |
| `target/` | Repoda yok, `cargo build` ile derlenir |
| Veritabanı şeması | `setup.sh` çalıştırılınca uygulama açılışta tabloları otomatik oluşturur |
| Şifre sıfırlama | `DELETE FROM sessions; DELETE FROM users;` → yeniden başlat → yeni admin seed'lenir |
