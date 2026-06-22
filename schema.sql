-- Uygulama bu tabloları ilk açılışta OTOMATİK oluşturur/günceller.
-- Referans:

CREATE DATABASE IF NOT EXISTS qrdb CHARACTER SET utf8mb4;
USE qrdb;

CREATE TABLE IF NOT EXISTS codes (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    code VARCHAR(255) NOT NULL UNIQUE,
    payload TEXT NULL,                 -- açıklama / link
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Bir koda bağlı detay alanları (metin / sayı / fotoğraf)
CREATE TABLE IF NOT EXISTS code_fields (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    code_id BIGINT NOT NULL,
    title VARCHAR(255) NOT NULL,
    field_type VARCHAR(20) NOT NULL,   -- 'text' | 'number' | 'image'
    value_text TEXT NULL,              -- text/number için
    image_data LONGBLOB NULL,          -- image için ikili veri
    image_mime VARCHAR(100) NULL,
    sort_order INT NOT NULL DEFAULT 0,
    FOREIGN KEY (code_id) REFERENCES codes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS users (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(100) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,  -- argon2
    is_admin TINYINT NOT NULL DEFAULT 0,
    must_change_password TINYINT NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sessions (
    token VARCHAR(64) PRIMARY KEY,
    user_id BIGINT NOT NULL,
    expires_at DATETIME NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
