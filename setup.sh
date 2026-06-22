#!/usr/bin/env bash
# =============================================================
# QR Kod Sistemi — Sıfırdan Kurulum Scripti
# Desteklenen: Ubuntu 22/24, Debian 12, Arch Linux
# Çalıştır:  bash setup.sh
# =============================================================
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()  { echo -e "${BLUE}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[ OK ]${NC}  $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error() { echo -e "${RED}[ERR ]${NC}  $*"; exit 1; }

echo ""
echo "=================================================="
echo "  QR Kod Sistemi — Kurulum"
echo "=================================================="
echo ""

# ─── 1. İşletim sistemi tespiti ───────────────────────────────
if [ -f /etc/os-release ]; then source /etc/os-release; else error "İşletim sistemi tespit edilemedi."; fi
OS="${ID:-unknown}"
info "İşletim sistemi: $PRETTY_NAME"

pkg_install() {
  case "$OS" in
    ubuntu|debian) sudo apt-get install -y "$@" ;;
    arch|cachyos|endeavouros|manjaro) sudo pacman -S --noconfirm "$@" ;;
    *) warn "Bilinmeyen dağıtım ($OS). Paketleri elle kur: $*" ;;
  esac
}

# ─── 2. Rust ──────────────────────────────────────────────────
info "Rust kontrol ediliyor..."
if ! command -v cargo &>/dev/null; then
  info "Rust bulunamadı, rustup ile kuruluyor..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  source "$HOME/.cargo/env"
  ok "Rust kuruldu: $(rustc --version)"
else
  ok "Rust mevcut: $(rustc --version)"
fi

# ─── 3. Gerekli sistem paketleri ──────────────────────────────
info "Sistem bağımlılıkları kontrol ediliyor..."
PKGS_MISSING=()

command -v mkcert &>/dev/null || PKGS_MISSING+=("mkcert")
command -v mysql  &>/dev/null || command -v mariadb &>/dev/null || PKGS_MISSING+=("mariadb" "mariadb-clients" 2>/dev/null || true)

case "$OS" in
  ubuntu|debian)
    dpkg -l libssl-dev &>/dev/null || PKGS_MISSING+=("libssl-dev")
    dpkg -l pkg-config &>/dev/null || PKGS_MISSING+=("pkg-config")
    dpkg -l nss-tools  &>/dev/null || PKGS_MISSING+=("libnss3-tools")
    ;;
  arch|cachyos|endeavouros|manjaro)
    pacman -Q nss &>/dev/null || PKGS_MISSING+=("nss")
    ;;
esac

if [ ${#PKGS_MISSING[@]} -gt 0 ]; then
  info "Eksik paketler kuruluyor: ${PKGS_MISSING[*]}"
  pkg_install "${PKGS_MISSING[@]}" || warn "Bazı paketler kurulamadı, devam ediliyor..."
fi

# Arch'ta MariaDB binary adı farklı olabilir
MYSQL_CMD="mysql"
command -v mysql   &>/dev/null || MYSQL_CMD="mariadb"
command -v $MYSQL_CMD &>/dev/null || error "mysql / mariadb komutu bulunamadı. MariaDB/MySQL kur ve tekrar dene."

# ─── 4. MariaDB servisi ───────────────────────────────────────
info "MariaDB servisi kontrol ediliyor..."
if ! systemctl is-active --quiet mariadb 2>/dev/null && ! systemctl is-active --quiet mysql 2>/dev/null; then
  warn "MariaDB servisi çalışmıyor, başlatılıyor..."
  sudo systemctl enable --now mariadb 2>/dev/null || sudo systemctl enable --now mysql 2>/dev/null || \
    warn "Servis başlatılamadı. Lütfen MariaDB'yi elle başlat: sudo systemctl start mariadb"
fi

# ─── 5. .env dosyası ──────────────────────────────────────────
if [ ! -f .env ]; then
  cp .env.example .env
  info ".env.example → .env kopyalandı."
fi

echo ""
echo "── Veritabanı & Admin Ayarları ──────────────────────"

# .env'den mevcut değerleri oku
source_env() {
  set -a
  # shellcheck disable=SC1091
  [ -f .env ] && source .env
  set +a
}
source_env

# Kullanıcıdan bilgi al (varsa mevcut değeri göster)
prompt_val() {
  local var="$1" prompt="$2" default="$3"
  local cur="${!var:-$default}"
  read -rp "  $prompt [$cur]: " tmp
  echo "${tmp:-$cur}"
}

DB_USER=$(prompt_val DB_USER "Veritabanı kullanıcı adı" "qruser")
DB_PASS=$(prompt_val DB_PASS "Veritabanı şifresi" "guclu-sifre")
DB_NAME=$(prompt_val DB_NAME "Veritabanı adı" "qrdb")
ADMIN_USER=$(prompt_val ADMIN_USER "Admin kullanıcı adı" "admin")
ADMIN_PASS=$(prompt_val ADMIN_PASS "Admin şifresi (ilk giriş için)" "degistir-beni")

# LAN IP otomatik tahmin
DETECTED_IP=$(ip route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[\d.]+' || hostname -I 2>/dev/null | awk '{print $1}' || echo "192.168.X.X")
BASE_URL=$(prompt_val BASE_URL "BASE_URL (QR'lara basılacak adres)" "https://${DETECTED_IP}:3000")

# .env'yi yaz
cat > .env << ENVEOF
DATABASE_URL=mysql://${DB_USER}:${DB_PASS}@localhost:3306/${DB_NAME}
ADMIN_USER=${ADMIN_USER}
ADMIN_PASS=${ADMIN_PASS}
BASE_URL=${BASE_URL}
ENVEOF
ok ".env güncellendi."

# ─── 6. Veritabanı & kullanıcı oluştur ───────────────────────
echo ""
info "Veritabanı oluşturuluyor (sudo mysql gerekebilir)..."
sudo $MYSQL_CMD << SQLEOF
CREATE DATABASE IF NOT EXISTS \`${DB_NAME}\` CHARACTER SET utf8mb4;
CREATE USER IF NOT EXISTS '${DB_USER}'@'localhost' IDENTIFIED BY '${DB_PASS}';
GRANT ALL PRIVILEGES ON \`${DB_NAME}\`.* TO '${DB_USER}'@'localhost';
FLUSH PRIVILEGES;
SQLEOF
ok "Veritabanı ve kullanıcı hazır: ${DB_NAME} / ${DB_USER}"

# ─── 7. TLS Sertifikası (mkcert) ──────────────────────────────
echo ""
info "TLS sertifikası oluşturuluyor..."
mkcert -install 2>/dev/null || warn "mkcert -install başarısız (sistem CA). Tarayıcı uyarı verebilir."
mkdir -p certs

# IP / hostname ayıkla
DOMAIN=$(echo "$BASE_URL" | sed 's|https://||' | cut -d: -f1)
mkcert -cert-file certs/cert.pem -key-file certs/key.pem localhost "127.0.0.1" "$DOMAIN" 2>/dev/null \
  || mkcert -cert-file certs/cert.pem -key-file certs/key.pem localhost 127.0.0.1 2>/dev/null
ok "Sertifika oluşturuldu: certs/cert.pem + certs/key.pem"

# ─── 8. Rust derle ────────────────────────────────────────────
echo ""
info "Proje derleniyor (ilk seferde birkaç dakika sürebilir)..."
source "$HOME/.cargo/env" 2>/dev/null || true
cargo build --release
ok "Derleme tamamlandı: target/release/qr-system"

# ─── 9. systemd servisi (opsiyonel) ───────────────────────────
echo ""
SERVICE_FILE="/etc/systemd/system/qr-system.service"
INSTALL_SERVICE="h"
read -rp "  systemd servisi oluşturulsun mu? (e/H): " INSTALL_SERVICE
INSTALL_SERVICE="${INSTALL_SERVICE:-h}"

if [[ "$INSTALL_SERVICE" =~ ^[eEyY] ]]; then
  WORK_DIR="$(pwd)"
  ENV_LINE="$(grep -v '^#' .env | xargs | tr ' ' '\n' | paste -sd ' ')"
  sudo tee "$SERVICE_FILE" > /dev/null << SVCEOF
[Unit]
Description=QR Kod Sistemi
After=network.target mariadb.service mysql.service
Wants=mariadb.service

[Service]
Type=simple
User=$(whoami)
WorkingDirectory=${WORK_DIR}
ExecStart=${WORK_DIR}/target/release/qr-system
Restart=on-failure
RestartSec=5
Environment="DATABASE_URL=mysql://${DB_USER}:${DB_PASS}@localhost:3306/${DB_NAME}"
Environment="ADMIN_USER=${ADMIN_USER}"
Environment="ADMIN_PASS=${ADMIN_PASS}"
Environment="BASE_URL=${BASE_URL}"

[Install]
WantedBy=multi-user.target
SVCEOF
  sudo systemctl daemon-reload
  sudo systemctl enable --now qr-system
  ok "Servis kuruldu ve başlatıldı: sudo systemctl status qr-system"
else
  info "Servis kurulmadı. Manuel çalıştırmak için:"
fi

# ─── 10. Özet ─────────────────────────────────────────────────
echo ""
echo "=================================================="
echo -e "  ${GREEN}Kurulum tamamlandı!${NC}"
echo "=================================================="
echo ""
echo "  Manuel çalıştır:"
echo "    source .env && cargo run --release"
echo "    # veya"
echo "    export \$(cat .env | xargs) && ./target/release/qr-system"
echo ""
echo "  Adresler:"
echo "    Yönetim  → ${BASE_URL}"
echo "    Tarama   → ${BASE_URL}/scan"
echo ""
echo "  İlk giriş → ${ADMIN_USER} / ${ADMIN_PASS}"
echo ""
echo "  Telefon kamerasıyla açılması için BASE_URL doğru mu kontrol et:"
echo "    ${BASE_URL}"
echo ""
