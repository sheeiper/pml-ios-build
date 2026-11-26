use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{four_domains, AppHandle};

const DEFAULT_JM_API_DOMAINS: &[&str] = &[
    "www.cdnhjk.net",
    "www.cdngwc.cc",
    "www.cdngwc.net",
    "www.cdngwc.club",
];

const DEFAULT_JM_IMG_DOMAINS: &[&str] = &[
    "cdn-msp.jmapiproxy1.cc",
    "cdn-msp.jmapiproxy2.cc",
    "cdn-msp2.jmapiproxy2.cc",
    "cdn-msp3.jmapiproxy2.cc",
    "cdn-msp.jmapinodeudzn.net",
    "cdn-msp3.jmapinodeudzn.net",
];

const PICA_API_BASE: &str = "https://picaapi.picacomic.com/";
const PICA_API_KEY: &str = "C69BAF41DA5ABD1FFEDC6D2FEA56B";
const PICA_HMAC_SECRET: &str = "~d}$Q7$eIni=V)9\\RK/P.RM4;9[7|@/CA}b~OW!3?EV`:<>M7pddUBL5n|0/*Cn";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cache_dir() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var("PML_DATA_DIR").unwrap_or_else(|_| "/data".to_string()),
    )
    .join("cache")
    .join("online_comics");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// ============================================================================
// Pure-Rust Crypto Primitives (Zero External Crate Dependencies)
// ============================================================================

fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
        5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];

    let mut msg = input.to_vec();
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            let sum = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(S[i]));
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hashed = Sha256::digest(key);
        k[..32].copy_from_slice(&hashed);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut i_pad = [0x36u8; 64];
    let mut o_pad = [0x5cu8; 64];
    for i in 0..64 {
        i_pad[i] ^= k[i];
        o_pad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(&i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&o_pad);
    outer.update(&inner_hash);
    let final_hash = outer.finalize();
    final_hash.iter().map(|b| format!("{b:02x}")).collect()
}

// FIPS-197 AES-256-ECB Decryption (for JMComic App API responses)
const AES_SBOX: [u8; 256] = [
    0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,
    0xca,0x82,0xc9,0x7d,0xfa,0x59,0x47,0xf0,0xad,0xd4,0xa2,0xaf,0x9c,0xa4,0x72,0xc0,
    0xb7,0xfd,0x93,0x26,0x36,0x3f,0xf7,0xcc,0x34,0xa5,0xe5,0xf1,0x71,0xd8,0x31,0x15,
    0x04,0xc7,0x23,0xc3,0x18,0x96,0x05,0x9a,0x07,0x12,0x80,0xe2,0xeb,0x27,0xb2,0x75,
    0x09,0x83,0x2c,0x1a,0x1b,0x6e,0x5a,0xa0,0x52,0x3b,0xd6,0xb3,0x29,0xe3,0x2f,0x84,
    0x53,0xd1,0x00,0xed,0x20,0xfc,0xb1,0x5b,0x6a,0xcb,0xbe,0x39,0x4a,0x4c,0x58,0xcf,
    0xd0,0xef,0xaa,0xfb,0x43,0x4d,0x33,0x85,0x45,0xf9,0x02,0x7f,0x50,0x3c,0x9f,0xa8,
    0x51,0xa3,0x40,0x8f,0x92,0x9d,0x38,0xf5,0xbc,0xb6,0xda,0x21,0x10,0xff,0xf3,0xd2,
    0xcd,0x0c,0x13,0xec,0x5f,0x97,0x44,0x17,0xc4,0xa7,0x7e,0x3d,0x64,0x5d,0x19,0x73,
    0x60,0x81,0x4f,0xdc,0x22,0x2a,0x90,0x88,0x46,0xee,0xb8,0x14,0xde,0x5e,0x0b,0xdb,
    0xe0,0x32,0x3a,0x0a,0x49,0x06,0x24,0x5c,0xc2,0xd3,0xac,0x62,0x91,0x95,0xe4,0x79,
    0xe7,0xc8,0x37,0x6d,0x8d,0xd5,0x4e,0xa9,0x6c,0x56,0xf4,0xea,0x65,0x7a,0xae,0x08,
    0xba,0x78,0x25,0x2e,0x1c,0xa6,0xb4,0xc6,0xe8,0xdd,0x74,0x1f,0x4b,0xbd,0x8b,0x8a,
    0x70,0x3e,0xb5,0x66,0x48,0x03,0xf6,0x0e,0x61,0x35,0x57,0xb9,0x86,0xc1,0x1d,0x9e,
    0xe1,0xf8,0x98,0x11,0x69,0xd9,0x8e,0x94,0x9b,0x1e,0x87,0xe9,0xce,0x55,0x28,0xdf,
    0x8c,0xa1,0x89,0x0d,0xbf,0xe6,0x42,0x68,0x41,0x99,0x2d,0x0f,0xb0,0x54,0xbb,0x16,
];

const AES_INV_SBOX: [u8; 256] = [
    0x52,0x09,0x6a,0xd5,0x30,0x36,0xa5,0x38,0xbf,0x40,0xa3,0x9e,0x81,0xf3,0xd7,0xfb,
    0x7c,0xe3,0x39,0x82,0x9b,0x2f,0xff,0x87,0x34,0x8e,0x43,0x44,0xc4,0xde,0xe9,0xcb,
    0x54,0x7b,0x94,0x32,0xa6,0xc2,0x23,0x3d,0xee,0x4c,0x95,0x0b,0x42,0xfa,0xc3,0x4e,
    0x08,0x2e,0xa1,0x66,0x28,0xd9,0x24,0xb2,0x76,0x5b,0xa2,0x49,0x6d,0x8b,0xd1,0x25,
    0x72,0xf8,0xf6,0x64,0x86,0x68,0x98,0x16,0xd4,0xa4,0x5c,0xcc,0x5d,0x65,0xb6,0x92,
    0x6c,0x70,0x48,0x50,0xfd,0xed,0xb9,0xda,0x5e,0x15,0x46,0x57,0xa7,0x8d,0x9d,0x84,
    0x90,0xd8,0xab,0x00,0x8c,0xbc,0xd3,0x0a,0xf7,0xe4,0x58,0x05,0xb8,0xb3,0x45,0x06,
    0xd0,0x2c,0x1e,0x8f,0xca,0x3f,0x0f,0x02,0xc1,0xaf,0xbd,0x03,0x01,0x13,0x8a,0x6b,
    0x3a,0x91,0x11,0x41,0x4f,0x67,0xdc,0xea,0x97,0xf2,0xcf,0xce,0xf0,0xb4,0xe6,0x73,
    0x96,0xac,0x74,0x22,0xe7,0xad,0x35,0x85,0xe2,0xf9,0x37,0xe8,0x1c,0x75,0xdf,0x6e,
    0x47,0xf1,0x1a,0x71,0x1d,0x29,0xc5,0x89,0x6f,0xb7,0x62,0x0e,0xaa,0x18,0xbe,0x1b,
    0xfc,0x56,0x3e,0x4b,0xc6,0xd2,0x79,0x20,0x9a,0xdb,0xc0,0xfe,0x78,0xcd,0x5a,0xf4,
    0x1f,0xdd,0xa8,0x33,0x88,0x07,0xc7,0x31,0xb1,0x12,0x10,0x59,0x27,0x80,0xec,0x5f,
    0x60,0x51,0x7f,0xa9,0x19,0xb5,0x4a,0x0d,0x2d,0xe5,0x7a,0x9f,0x93,0xc9,0x9c,0xef,
    0xa0,0xe0,0x3b,0x4d,0xae,0x2a,0xf5,0xb0,0xc8,0xeb,0xbb,0x3c,0x83,0x53,0x99,0x61,
    0x17,0x2b,0x04,0x7e,0xba,0x77,0xd6,0x26,0xe1,0x69,0x14,0x63,0x55,0x21,0x0c,0x7d,
];

fn xtime(x: u8) -> u8 {
    (x << 1) ^ if (x & 0x80) != 0 { 0x1b } else { 0 }
}

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

fn aes256_expand_key(key: &[u8; 32]) -> [[u8; 16]; 15] {
    const RCON: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];
    let mut w = [[0u8; 4]; 60];
    for i in 0..8 {
        w[i] = [key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]];
    }
    for i in 8..60 {
        let mut temp = w[i - 1];
        if i % 8 == 0 {
            temp = [
                AES_SBOX[temp[1] as usize] ^ RCON[i / 8 - 1],
                AES_SBOX[temp[2] as usize],
                AES_SBOX[temp[3] as usize],
                AES_SBOX[temp[0] as usize],
            ];
        } else if i % 8 == 4 {
            temp = [
                AES_SBOX[temp[0] as usize],
                AES_SBOX[temp[1] as usize],
                AES_SBOX[temp[2] as usize],
                AES_SBOX[temp[3] as usize],
            ];
        }
        for j in 0..4 {
            w[i][j] = w[i - 8][j] ^ temp[j];
        }
    }
    let mut rk = [[0u8; 16]; 15];
    for r in 0..15 {
        for c in 0..4 {
            for row in 0..4 {
                rk[r][c * 4 + row] = w[r * 4 + c][row];
            }
        }
    }
    rk
}

fn aes256_decrypt_block(rk: &[[u8; 16]; 15], block: &[u8]) -> [u8; 16] {
    let mut s = [0u8; 16];
    s.copy_from_slice(block);
    for i in 0..16 {
        s[i] ^= rk[14][i];
    }
    for round in (1..14).rev() {
        // InvShiftRows
        let t = s;
        s[1] = t[13]; s[5] = t[1]; s[9] = t[5]; s[13] = t[9];
        s[2] = t[10]; s[6] = t[14]; s[10] = t[2]; s[14] = t[6];
        s[3] = t[7]; s[7] = t[11]; s[11] = t[15]; s[15] = t[3];
        // InvSubBytes
        for b in &mut s {
            *b = AES_INV_SBOX[*b as usize];
        }
        // AddRoundKey
        for i in 0..16 {
            s[i] ^= rk[round][i];
        }
        // InvMixColumns
        for c in 0..4 {
            let i = c * 4;
            let (s0, s1, s2, s3) = (s[i], s[i + 1], s[i + 2], s[i + 3]);
            s[i]     = gf_mul(s0, 14) ^ gf_mul(s1, 11) ^ gf_mul(s2, 13) ^ gf_mul(s3, 9);
            s[i + 1] = gf_mul(s0, 9)  ^ gf_mul(s1, 14) ^ gf_mul(s2, 11) ^ gf_mul(s3, 13);
            s[i + 2] = gf_mul(s0, 13) ^ gf_mul(s1, 9)  ^ gf_mul(s2, 14) ^ gf_mul(s3, 11);
            s[i + 3] = gf_mul(s0, 11) ^ gf_mul(s1, 13) ^ gf_mul(s2, 9)  ^ gf_mul(s3, 14);
        }
    }
    // Final round
    let t = s;
    s[1] = t[13]; s[5] = t[1]; s[9] = t[5]; s[13] = t[9];
    s[2] = t[10]; s[6] = t[14]; s[10] = t[2]; s[14] = t[6];
    s[3] = t[7]; s[7] = t[11]; s[11] = t[15]; s[15] = t[3];
    for b in &mut s {
        *b = AES_INV_SBOX[*b as usize];
    }
    for i in 0..16 {
        s[i] ^= rk[0][i];
    }
    s
}

fn aes256_ecb_decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err("密文长度无效".to_string());
    }
    let rk = aes256_expand_key(key);
    let mut out = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks_exact(16) {
        out.extend_from_slice(&aes256_decrypt_block(&rk, chunk));
    }
    if let Some(&pad) = out.last() {
        let p = pad as usize;
        if p >= 1 && p <= 16 && out.len() >= p && out[out.len() - p..].iter().all(|&b| b == pad) {
            out.truncate(out.len() - p);
        }
    }
    Ok(out)
}

// ============================================================================
// Data Models & Config Storage
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineComicConfigView {
    pub jm_domains: Vec<String>,
    pub jm_image_domains: Vec<String>,
    pub pica_email: String,
    pub pica_has_password: bool,
    pub pica_logged_in: bool,
    pub pica_channel: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOnlineComicConfigRequest {
    pub jm_domains: Option<Vec<String>>,
    pub jm_image_domains: Option<Vec<String>>,
    pub pica_email: Option<String>,
    pub pica_password: Option<String>,
    pub pica_channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineComicItem {
    pub source: String, // "jm" | "pica"
    pub id: String,
    pub title: String,
    pub author: String,
    pub cover_url: String,
    pub category: String,
    pub tags: Vec<String>,
    pub likes: Option<i64>,
    pub views: Option<i64>,
    pub pages_count: Option<i64>,
    pub finished: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineComicPageResult {
    pub source: String,
    pub items: Vec<OnlineComicItem>,
    pub page: i64,
    pub has_more: bool,
    pub total: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineComicChapter {
    pub id: String,
    pub order: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineComicDetail {
    pub source: String,
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub cover_url: String,
    pub description: String,
    pub tags: Vec<String>,
    pub likes: Option<i64>,
    pub views: Option<i64>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub is_favourite: Option<bool>,
    pub chapters: Vec<OnlineComicChapter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineChapterPagesResult {
    pub source: String,
    pub comic_id: String,
    pub chapter_id: String,
    pub images: Vec<String>,
}

struct StoredOnlineConfig {
    jm_domains: Vec<String>,
    jm_image_domains: Vec<String>,
    pica_email: String,
    pica_password: String,
    pica_token: String,
    pica_channel: String,
}

fn ensure_config_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS online_comic_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())
}

fn get_kv(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM online_comic_config WHERE key = ?1", params![key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

fn set_kv(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO online_comic_config (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn load_stored_config(conn: &Connection) -> StoredOnlineConfig {
    let parse_list = |key: &str, defaults: &[&str]| -> Vec<String> {
        if let Some(raw) = get_kv(conn, key) {
            let list: Vec<String> = raw
                .split(|c| c == ',' || c == '\n')
                .map(|s| s.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !list.is_empty() {
                return list;
            }
        }
        defaults.iter().map(|s| (*s).to_string()).collect()
    };
    StoredOnlineConfig {
        jm_domains: parse_list("jm_domains", DEFAULT_JM_API_DOMAINS),
        jm_image_domains: parse_list("jm_image_domains", DEFAULT_JM_IMG_DOMAINS),
        pica_email: get_kv(conn, "pica_email").unwrap_or_default(),
        pica_password: get_kv(conn, "pica_password").unwrap_or_default(),
        pica_token: get_kv(conn, "pica_token").unwrap_or_default(),
        pica_channel: get_kv(conn, "pica_channel").unwrap_or_else(|| "1".to_string()),
    }
}

pub fn get_online_comic_config(app: &AppHandle) -> Result<OnlineComicConfigView, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let cfg = load_stored_config(&conn);
    Ok(OnlineComicConfigView {
        jm_domains: cfg.jm_domains,
        jm_image_domains: cfg.jm_image_domains,
        pica_email: cfg.pica_email,
        pica_has_password: !cfg.pica_password.is_empty(),
        pica_logged_in: !cfg.pica_token.is_empty(),
        pica_channel: cfg.pica_channel,
    })
}

pub fn save_online_comic_config(app: &AppHandle, req: SaveOnlineComicConfigRequest) -> Result<OnlineComicConfigView, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;

    if let Some(domains) = req.jm_domains {
        let joined = domains
            .into_iter()
            .map(|s| s.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        set_kv(&conn, "jm_domains", &joined)?;
    }
    if let Some(img_domains) = req.jm_image_domains {
        let joined = img_domains
            .into_iter()
            .map(|s| s.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        set_kv(&conn, "jm_image_domains", &joined)?;
    }
    if let Some(ch) = req.pica_channel {
        let c = match ch.trim() {
            "2" => "2",
            "3" => "3",
            _ => "1",
        };
        set_kv(&conn, "pica_channel", c)?;
    }

    let mut need_pica_login = false;
    if let Some(email) = req.pica_email {
        set_kv(&conn, "pica_email", email.trim())?;
        need_pica_login = true;
    }
    if let Some(pw) = req.pica_password {
        if !pw.is_empty() {
            set_kv(&conn, "pica_password", &pw)?;
            need_pica_login = true;
        }
    }

    let cfg = load_stored_config(&conn);
    if need_pica_login && !cfg.pica_email.is_empty() && !cfg.pica_password.is_empty() {
        let token = pica_login_blocking(&cfg.pica_email, &cfg.pica_password, &cfg.pica_channel)?;
        set_kv(&conn, "pica_token", &token)?;
    }

    get_online_comic_config(app)
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(18))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))
}

// ============================================================================
// JMComic (禁漫天堂) App API Client
// ============================================================================

fn jm_get_json_blocking(domains: &[String], path_and_query: &str) -> Result<Value, String> {
    let rt = tokio::runtime::Handle::try_current()
        .ok();
    let fut = jm_get_json_async(domains.to_vec(), path_and_query.to_string());
    if let Some(handle) = rt {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(fut)
    }
}

async fn jm_get_json_async(domains: Vec<String>, path_and_query: String) -> Result<Value, String> {
    let client = http_client()?;
    let mut last_err = String::from("无可用 JMComic 域名");

    for domain in &domains {
        let ts = now_secs();
        let token = md5_hex(format!("{ts}185Hcomic3PAPP7R").as_bytes());
        let tokenparam = format!("{ts},2.1.13");
        let url = format!("https://{domain}{path_and_query}");

        let resp = match client
            .get(&url)
            .header("token", &token)
            .header("tokenparam", &tokenparam)
            .header("User-Agent", "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Mobile Safari/537.36")
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{domain}: {e}");
                continue;
            }
        };

        if !resp.status().is_success() {
            last_err = format!("{domain}: HTTP {}", resp.status());
            continue;
        }
        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("{domain}: JSON 解析失败 {e}");
                continue;
            }
        };

        if let Some(enc_b64) = body.get("data").and_then(Value::as_str) {
            let cipher = match STANDARD.decode(enc_b64.trim()) {
                Ok(b) => b,
                Err(e) => {
                    last_err = format!("{domain}: base64 解码失败 {e}");
                    continue;
                }
            };
            let key_hex = md5_hex(format!("{ts}185Hcomic3PAPP7R").as_bytes());
            let mut key = [0u8; 32];
            key.copy_from_slice(&key_hex.as_bytes()[..32]);
            let plain = match aes256_ecb_decrypt(&key, &cipher) {
                Ok(p) => p,
                Err(e) => {
                    last_err = format!("{domain}: AES 解密失败 {e}");
                    continue;
                }
            };
            match serde_json::from_slice::<Value>(&plain) {
                Ok(parsed) => return Ok(parsed),
                Err(e) => {
                    last_err = format!("{domain}: 解密后 JSON 无效 {e}");
                    continue;
                }
            }
        } else if let Some(data_val) = body.get("data") {
            return Ok(data_val.clone());
        } else {
            last_err = format!("{domain}: 响应缺少 data 字段");
        }
    }
    Err(format!("JMComic 请求失败（已尝试 {} 个域名）：{last_err}", domains.len()))
}

fn jm_cover_proxy_url(album_id: &str) -> String {
    format!("/api/online-comic-image?source=jm_cover&albumId={}", urlencoding(album_id))
}

fn jm_page_proxy_url(photo_id: &str, img: &str) -> String {
    format!(
        "/api/online-comic-image?source=jm&photoId={}&img={}",
        urlencoding(photo_id),
        urlencoding(img)
    )
}

fn pica_img_proxy_url(file_server: &str, path: &str) -> String {
    let base = file_server.trim_end_matches('/');
    let p = path.trim_start_matches('/');
    let full = if p.starts_with("static/") {
        format!("{base}/{p}")
    } else {
        format!("{base}/static/{p}")
    };
    format!("/api/online-comic-image?source=pica&url={}", urlencoding(&full))
}

fn urlencoding(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn parse_jm_item(v: &Value) -> Option<OnlineComicItem> {
    let id = v.get("id").and_then(|x| {
        x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string()))
    })?;
    let title = v.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if title.is_empty() {
        return None;
    }
    let author = v.get("author").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let category = v
        .get("category")
        .and_then(|c| c.get("title").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let likes = v.get("liked").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
    Some(OnlineComicItem {
        source: "jm".to_string(),
        cover_url: jm_cover_proxy_url(&id),
        id,
        title,
        author,
        category,
        tags: Vec::new(),
        likes,
        views: None,
        pages_count: None,
        finished: None,
    })
}

fn browse_jm(
    cfg: &StoredOnlineConfig,
    mode: Option<&str>,
    query: Option<&str>,
    category: Option<&str>,
    sort: Option<&str>,
    page: i64,
) -> Result<OnlineComicPageResult, String> {
    let page = page.max(1);
    let mode_str = mode.unwrap_or("browse");

    // Rankings & Random discovery
    if mode_str == "rank_day" || mode_str == "rank_week" || mode_str == "rank_month" || mode_str == "rank_all" || mode_str == "rank_likes" {
        let o = match mode_str {
            "rank_day" => "mv_d",
            "rank_week" => "mv_w",
            "rank_month" => "mv_m",
            "rank_likes" => "tf",
            _ => "mv",
        };
        let path = format!("/categories/filter?page={page}&o={o}&c=0");
        let data = jm_get_json_blocking(&cfg.jm_domains, &path)?;
        let items: Vec<OnlineComicItem> = data
            .get("content")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_jm_item).collect())
            .unwrap_or_default();
        let total = data.get("total").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
        return Ok(OnlineComicPageResult {
            source: "jm".to_string(),
            has_more: items.len() >= 20,
            items,
            page,
            total,
        });
    }
    if mode_str == "random" {
        let rand_page = (rand::random::<u64>() % 30) as i64 + 1;
        let path = format!("/categories/filter?page={rand_page}&o=tf&c=0");
        let data = jm_get_json_blocking(&cfg.jm_domains, &path)?;
        let items: Vec<OnlineComicItem> = data
            .get("content")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_jm_item).collect())
            .unwrap_or_default();
        return Ok(OnlineComicPageResult {
            source: "jm".to_string(),
            has_more: true,
            items,
            page: 1,
            total: None,
        });
    }

    let order = match sort.unwrap_or("mr") {
        "mv" | "views" => "mv",
        "mv_d" => "mv_d",
        "mv_w" => "mv_w",
        "mv_m" => "mv_m",
        "mp" | "pages" => "mp",
        "tf" | "likes" => "tf",
        _ => "mr",
    };

    // If user picked an extra theme/tag category (not one of the 6 main JM slugs), route via JM tag search!
    let main_cat_slug = match category.unwrap_or("全部") {
        "" | "全部" | "0" => Some("0"),
        "同人" | "doujin" => Some("doujin"),
        "单本" | "single" => Some("single"),
        "短篇" | "short" => Some("short"),
        "韩漫" | "hanman" => Some("hanman"),
        "美漫" | "meiman" => Some("meiman"),
        "其他" | "another" => Some("another"),
        _ => None,
    };

    let effective_query = query
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| if main_cat_slug.is_none() { category.map(str::trim).filter(|s| !s.is_empty()) } else { None });

    if let Some(q) = effective_query {
        // Direct JM numeric ID lookup
        if q.chars().all(|c| c.is_ascii_digit()) && q.len() >= 4 {
            if let Ok(detail) = get_jm_detail(cfg, q) {
                return Ok(OnlineComicPageResult {
                    source: "jm".to_string(),
                    items: vec![OnlineComicItem {
                        source: "jm".to_string(),
                        id: detail.id,
                        title: detail.title,
                        author: detail.authors.first().cloned().unwrap_or_default(),
                        cover_url: detail.cover_url,
                        category: "JM车牌".to_string(),
                        tags: detail.tags,
                        likes: detail.likes,
                        views: detail.views,
                        pages_count: None,
                        finished: None,
                    }],
                    page: 1,
                    has_more: false,
                    total: Some(1),
                });
            }
        }
        let path = format!("/search?search_query={}&page={page}&o={order}", urlencoding(q));
        let data = jm_get_json_blocking(&cfg.jm_domains, &path)?;
        if let Some(redirect_aid) = data.get("redirect_aid").and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string()))) {
            if !redirect_aid.is_empty() && redirect_aid != "0" {
                if let Ok(detail) = get_jm_detail(cfg, &redirect_aid) {
                    return Ok(OnlineComicPageResult {
                        source: "jm".to_string(),
                        items: vec![OnlineComicItem {
                            source: "jm".to_string(),
                            id: detail.id,
                            title: detail.title,
                            author: detail.authors.first().cloned().unwrap_or_default(),
                            cover_url: detail.cover_url,
                            category: "JM直达".to_string(),
                            tags: detail.tags,
                            likes: detail.likes,
                            views: detail.views,
                            pages_count: None,
                            finished: None,
                        }],
                        page: 1,
                        has_more: false,
                        total: Some(1),
                    });
                }
            }
        }
        let items: Vec<OnlineComicItem> = data
            .get("content")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_jm_item).collect())
            .unwrap_or_default();
        let total = data.get("total").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
        let has_more = items.len() >= 20;
        return Ok(OnlineComicPageResult {
            source: "jm".to_string(),
            items,
            page,
            has_more,
            total,
        });
    }

    let cat = main_cat_slug.unwrap_or("0");
    let path = format!("/categories/filter?page={page}&o={order}&c={cat}");
    let data = jm_get_json_blocking(&cfg.jm_domains, &path)?;
    let items: Vec<OnlineComicItem> = data
        .get("content")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_jm_item).collect())
        .unwrap_or_default();
    let total = data.get("total").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
    let has_more = items.len() >= 20;
    Ok(OnlineComicPageResult {
        source: "jm".to_string(),
        items,
        page,
        has_more,
        total,
    })
}

fn get_jm_detail(cfg: &StoredOnlineConfig, album_id: &str) -> Result<OnlineComicDetail, String> {
    let data = jm_get_json_blocking(&cfg.jm_domains, &format!("/album?id={}", urlencoding(album_id)))?;
    let id = data
        .get("id")
        .and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string())))
        .unwrap_or_else(|| album_id.to_string());
    let title = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let authors: Vec<String> = data
        .get("author")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let tags: Vec<String> = data
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let description = data.get("description").and_then(Value::as_str).unwrap_or("").to_string();
    let likes = data.get("likes").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
    let views = data.get("total_views").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
    let mut chapters: Vec<OnlineComicChapter> = data
        .get("series")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(idx, ch)| {
                    let cid = ch.get("id").and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string())))?;
                    let sort = ch.get("sort").and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or((idx + 1) as i64);
                    let name = ch.get("name").and_then(Value::as_str).unwrap_or("").trim();
                    let label = if name.is_empty() { format!("第 {sort} 话") } else { format!("第 {sort} 话 · {name}") };
                    Some(OnlineComicChapter { id: cid, order: sort, title: label })
                })
                .collect()
        })
        .unwrap_or_default();

    if chapters.is_empty() {
        chapters.push(OnlineComicChapter {
            id: id.clone(),
            order: 1,
            title: "单话全本".to_string(),
        });
    }

    Ok(OnlineComicDetail {
        source: "jm".to_string(),
        cover_url: jm_cover_proxy_url(&id),
        id,
        title,
        authors,
        description,
        tags,
        likes,
        views,
        updated_at: None,
        is_favourite: None,
        chapters,
    })
}

fn get_jm_chapter_pages(cfg: &StoredOnlineConfig, album_id: &str, chapter_id: &str) -> Result<OnlineChapterPagesResult, String> {
    let data = jm_get_json_blocking(&cfg.jm_domains, &format!("/chapter?id={}", urlencoding(chapter_id)))?;
    let images: Vec<String> = data
        .get("images")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|img| jm_page_proxy_url(chapter_id, img))
                .collect()
        })
        .unwrap_or_default();
    Ok(OnlineChapterPagesResult {
        source: "jm".to_string(),
        comic_id: album_id.to_string(),
        chapter_id: chapter_id.to_string(),
        images,
    })
}

// ============================================================================
// PicACG (哔咔漫画) App API Client
// ============================================================================

fn pica_request_blocking(
    method: &str,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
    channel: &str,
) -> Result<Value, String> {
    let rt = tokio::runtime::Handle::try_current().ok();
    let fut = pica_request_async(
        method.to_string(),
        path.to_string(),
        body,
        token.map(str::to_string),
        channel.to_string(),
    );
    if let Some(handle) = rt {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(fut)
    }
}

async fn pica_request_async(
    method: String,
    path: String,
    body: Option<Value>,
    token: Option<String>,
    channel: String,
) -> Result<Value, String> {
    let client = http_client()?;
    let ts = now_secs().to_string();
    let nonce = md5_hex(format!("pml-{ts}-{}", rand::random::<u64>()).as_bytes());
    let clean_path = path.trim_start_matches('/');
    let raw = format!("{clean_path}{ts}{nonce}{method}{PICA_API_KEY}").to_lowercase();
    let sig = hmac_sha256_hex(PICA_HMAC_SECRET.as_bytes(), raw.as_bytes());
    let url = format!("{PICA_API_BASE}{clean_path}");

    let mut req = if method.eq_ignore_ascii_case("POST") {
        client.post(&url)
    } else {
        client.get(&url)
    };
    req = req
        .header("api-key", PICA_API_KEY)
        .header("accept", "application/vnd.picacomic.com.v1+json")
        .header("app-channel", &channel)
        .header("time", &ts)
        .header("nonce", &nonce)
        .header("signature", &sig)
        .header("app-version", "2.2.1.3.3.4")
        .header("app-uuid", "defaultUuid")
        .header("image-quality", "original")
        .header("app-platform", "android")
        .header("app-build-version", "45")
        .header("User-Agent", "okhttp/3.8.1");
    if let Some(tk) = token.filter(|s| !s.is_empty()) {
        req = req.header("authorization", tk);
    }
    if let Some(b) = body {
        req = req.header("Content-Type", "application/json; charset=UTF-8").json(&b);
    }

    let resp = req.send().await.map_err(|e| format!("哔咔请求失败：{e}"))?;
    let status = resp.status();
    let json: Value = resp.json().await.map_err(|e| format!("哔咔响应解析失败：{e}"))?;
    if !status.is_success() || json.get("code").and_then(Value::as_i64).unwrap_or(200) != 200 {
        let msg = json.get("message").and_then(Value::as_str).unwrap_or("未知错误");
        return Err(format!("哔咔接口错误 ({status}): {msg}"));
    }
    Ok(json.get("data").cloned().unwrap_or(Value::Null))
}

fn pica_login_blocking(email: &str, password: &str, channel: &str) -> Result<String, String> {
    let data = pica_request_blocking(
        "POST",
        "auth/sign-in",
        Some(serde_json::json!({ "email": email, "password": password })),
        None,
        channel,
    )?;
    data.get("token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "哔咔登录未返回 token，请检查账号密码".to_string())
}

fn ensure_pica_token(conn: &Connection, cfg: &mut StoredOnlineConfig) -> Result<String, String> {
    if !cfg.pica_token.is_empty() {
        return Ok(cfg.pica_token.clone());
    }
    if cfg.pica_email.is_empty() || cfg.pica_password.is_empty() {
        return Err("尚未配置哔咔漫画账号，请先在右上角「源设置」填入哔咔邮箱与密码".to_string());
    }
    let tk = pica_login_blocking(&cfg.pica_email, &cfg.pica_password, &cfg.pica_channel)?;
    let _ = set_kv(conn, "pica_token", &tk);
    cfg.pica_token = tk.clone();
    Ok(tk)
}

fn parse_pica_item(v: &Value) -> Option<OnlineComicItem> {
    let id = v.get("_id").and_then(Value::as_str)?.to_string();
    let title = v.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    if title.is_empty() {
        return None;
    }
    let author = v.get("author").and_then(Value::as_str).unwrap_or("").to_string();
    let thumb = v.get("thumb");
    let cover_url = thumb
        .and_then(|t| {
            let fs = t.get("fileServer").and_then(Value::as_str)?;
            let p = t.get("path").and_then(Value::as_str)?;
            Some(pica_img_proxy_url(fs, p))
        })
        .unwrap_or_default();
    let tags: Vec<String> = v
        .get("categories")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let category = tags.first().cloned().unwrap_or_default();
    Some(OnlineComicItem {
        source: "pica".to_string(),
        id,
        title,
        author,
        cover_url,
        category,
        tags,
        likes: v.get("totalLikes").or_else(|| v.get("likesCount")).and_then(Value::as_i64),
        views: v.get("totalViews").and_then(Value::as_i64),
        pages_count: v.get("pagesCount").and_then(Value::as_i64),
        finished: v.get("finished").and_then(Value::as_bool),
    })
}

fn browse_pica(
    conn: &Connection,
    cfg: &mut StoredOnlineConfig,
    mode: Option<&str>,
    query: Option<&str>,
    category: Option<&str>,
    sort: Option<&str>,
    page: i64,
) -> Result<OnlineComicPageResult, String> {
    let token = ensure_pica_token(conn, cfg)?;
    let page = page.max(1);
    let mode_str = mode.unwrap_or("browse");
    let s = match sort.unwrap_or("dd") {
        "da" => "da",
        "ld" | "likes" => "ld",
        "vd" | "views" => "vd",
        _ => "dd",
    };

    // PicACG Leaderboard (H24 / D7 / D30)
    if mode_str == "rank_day" || mode_str == "rank_week" || mode_str == "rank_month" || mode_str == "rank_all" {
        let tt = match mode_str {
            "rank_day" => "H24",
            "rank_week" => "D7",
            _ => "D30",
        };
        let data = pica_request_blocking(
            "GET",
            &format!("comics/leaderboard?tt={tt}&ct=VC"),
            None,
            Some(&token),
            &cfg.pica_channel,
        )?;
        let items: Vec<OnlineComicItem> = data
            .get("comics")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_pica_item).collect())
            .unwrap_or_default();
        return Ok(OnlineComicPageResult {
            source: "pica".to_string(),
            total: Some(items.len() as i64),
            items,
            page: 1,
            has_more: false,
        });
    }

    // PicACG Random
    if mode_str == "random" {
        let data = pica_request_blocking("GET", "comics/random", None, Some(&token), &cfg.pica_channel)?;
        let items: Vec<OnlineComicItem> = data
            .get("comics")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_pica_item).collect())
            .unwrap_or_default();
        return Ok(OnlineComicPageResult {
            source: "pica".to_string(),
            total: Some(items.len() as i64),
            items,
            page: 1,
            has_more: true,
        });
    }

    // PicACG Personal Favorites (我的哔咔收藏)
    if mode_str == "favorites" {
        let fav_sort = if s == "da" { "da" } else { "dd" };
        let data = pica_request_blocking(
            "GET",
            &format!("users/favourite?s={fav_sort}&page={page}"),
            None,
            Some(&token),
            &cfg.pica_channel,
        )?;
        let comics = data.get("comics").unwrap_or(&data);
        let items: Vec<OnlineComicItem> = comics
            .get("docs")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_pica_item).collect())
            .unwrap_or_default();
        let total = comics.get("total").and_then(Value::as_i64);
        let pages = comics.get("pages").and_then(Value::as_i64).unwrap_or(page);
        return Ok(OnlineComicPageResult {
            source: "pica".to_string(),
            items,
            page,
            has_more: page < pages,
            total,
        });
    }

    let data = if let Some(q) = query.map(str::trim).filter(|x| !x.is_empty()) {
        pica_request_blocking(
            "POST",
            &format!("comics/advanced-search?page={page}"),
            Some(serde_json::json!({ "keyword": q, "sort": s })),
            Some(&token),
            &cfg.pica_channel,
        )?
    } else if let Some(cat) = category.map(str::trim).filter(|c| !c.is_empty() && *c != "全部" && *c != "0") {
        pica_request_blocking(
            "GET",
            &format!("comics?page={page}&c={}&s={s}", urlencoding(cat)),
            None,
            Some(&token),
            &cfg.pica_channel,
        )?
    } else {
        pica_request_blocking(
            "GET",
            &format!("comics?page={page}&s={s}"),
            None,
            Some(&token),
            &cfg.pica_channel,
        )?
    };

    let comics = data.get("comics").unwrap_or(&data);
    let items: Vec<OnlineComicItem> = comics
        .get("docs")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_pica_item).collect())
        .unwrap_or_default();
    let total = comics.get("total").and_then(Value::as_i64);
    let pages = comics.get("pages").and_then(Value::as_i64).unwrap_or(page);
    Ok(OnlineComicPageResult {
        source: "pica".to_string(),
        items,
        page,
        has_more: page < pages,
        total,
    })
}

fn get_pica_detail(conn: &Connection, cfg: &mut StoredOnlineConfig, comic_id: &str) -> Result<OnlineComicDetail, String> {
    let token = ensure_pica_token(conn, cfg)?;
    let detail_data = pica_request_blocking("GET", &format!("comics/{comic_id}"), None, Some(&token), &cfg.pica_channel)?;
    let c = detail_data.get("comic").unwrap_or(&detail_data);
    let title = c.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let author = c.get("author").and_then(Value::as_str).unwrap_or("").to_string();
    let description = c.get("description").and_then(Value::as_str).unwrap_or("").to_string();
    let cover_url = c
        .get("thumb")
        .and_then(|t| Some(pica_img_proxy_url(t.get("fileServer")?.as_str()?, t.get("path")?.as_str()?)))
        .unwrap_or_default();
    let mut tags: Vec<String> = c
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if let Some(cats) = c.get("categories").and_then(Value::as_array) {
        for cat in cats.iter().filter_map(Value::as_str) {
            if !tags.iter().any(|t| t == cat) {
                tags.push(cat.to_string());
            }
        }
    }

    let eps_data = pica_request_blocking("GET", &format!("comics/{comic_id}/eps?page=1"), None, Some(&token), &cfg.pica_channel)?;
    let mut chapters: Vec<OnlineComicChapter> = eps_data
        .get("eps")
        .and_then(|e| e.get("docs"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|ep| {
                    let order = ep.get("order").and_then(Value::as_i64)?;
                    let t = ep.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                    Some(OnlineComicChapter {
                        id: order.to_string(),
                        order,
                        title: if t.is_empty() { format!("第 {order} 话") } else { t },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    chapters.sort_by_key(|ch| ch.order);

    Ok(OnlineComicDetail {
        source: "pica".to_string(),
        id: comic_id.to_string(),
        title,
        authors: if author.is_empty() { Vec::new() } else { vec![author] },
        cover_url,
        description,
        tags,
        likes: c.get("totalLikes").or_else(|| c.get("likesCount")).and_then(Value::as_i64),
        views: c.get("totalViews").and_then(Value::as_i64),
        updated_at: c.get("updated_at").and_then(Value::as_str).map(str::to_string),
        is_favourite: c.get("isFavourite").and_then(Value::as_bool),
        chapters,
    })
}

fn get_pica_chapter_pages(conn: &Connection, cfg: &mut StoredOnlineConfig, comic_id: &str, order: &str) -> Result<OnlineChapterPagesResult, String> {
    let token = ensure_pica_token(conn, cfg)?;
    let mut images = Vec::new();
    let mut cur_page = 1i64;
    loop {
        let data = pica_request_blocking(
            "GET",
            &format!("comics/{comic_id}/order/{order}/pages?page={cur_page}"),
            None,
            Some(&token),
            &cfg.pica_channel,
        )?;
        let pages_obj = data.get("pages").unwrap_or(&data);
        if let Some(docs) = pages_obj.get("docs").and_then(Value::as_array) {
            for doc in docs {
                if let Some(media) = doc.get("media") {
                    if let (Some(fs), Some(p)) = (
                        media.get("fileServer").and_then(Value::as_str),
                        media.get("path").and_then(Value::as_str),
                    ) {
                        images.push(pica_img_proxy_url(fs, p));
                    }
                }
            }
        }
        let total_pages = pages_obj.get("pages").and_then(Value::as_i64).unwrap_or(1);
        if cur_page >= total_pages || cur_page >= 15 {
            break;
        }
        cur_page += 1;
    }
    Ok(OnlineChapterPagesResult {
        source: "pica".to_string(),
        comic_id: comic_id.to_string(),
        chapter_id: order.to_string(),
        images,
    })
}

// ============================================================================
// Public Unified Dispatchers
// ============================================================================

pub fn list_online_comic_categories(app: &AppHandle, source: &str) -> Result<Vec<String>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let mut cfg = load_stored_config(&conn);

    if source == "pica" {
        let mut out = vec!["全部".to_string()];
        if let Ok(token) = ensure_pica_token(&conn, &mut cfg) {
            if let Ok(data) = pica_request_blocking("GET", "categories", None, Some(&token), &cfg.pica_channel) {
                if let Some(arr) = data.get("categories").and_then(Value::as_array) {
                    for item in arr {
                        if item.get("isWeb").and_then(Value::as_bool).unwrap_or(false) {
                            continue;
                        }
                        if let Some(t) = item.get("title").and_then(Value::as_str) {
                            let clean = t.trim();
                            if !clean.is_empty() && !out.iter().any(|x| x == clean) {
                                out.push(clean.to_string());
                            }
                        }
                    }
                }
            }
        }
        if out.len() <= 1 {
            for fallback in [
                "大家都在看", "大濕推薦", "那年今天", "官方都在看", "嗶咔漢化", "全彩",
                "長篇", "同人", "短篇", "單行本", "純愛", "校園", "人妻", "姐姐系", "妹妹系",
                "後宮閃光", "NTR", "百合花園", "扶他樂園", "CG雜圖", "Cosplay", "WEBTOON",
                "生肉", "英語 ENG", "禁書目錄", "碧藍幻想", "Fate", "東方", "歐美", "非人類",
            ] {
                out.push(fallback.to_string());
            }
        }
        return Ok(out);
    }

    // JMComic: Main categories + popular Chinese tags
    Ok([
        "全部", "同人", "单本", "短篇", "韩漫", "美漫", "其他",
        "汉化", "全彩", "无修正", "纯爱", "校园", "人妻", "姐妹", "青梅竹马",
        "辣妹", "巨乳", "NTR", "催眠", "百合", "扶她", "触手", "Cosplay", "CG图集", "3D", "完结",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect())
}

pub fn toggle_online_comic_favorite(app: &AppHandle, source: &str, comic_id: &str) -> Result<bool, String> {
    if source != "pica" {
        return Err("当前仅哔咔漫画支持账号同步收藏".to_string());
    }
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let mut cfg = load_stored_config(&conn);
    let token = ensure_pica_token(&conn, &mut cfg)?;
    let data = pica_request_blocking(
        "POST",
        &format!("comics/{comic_id}/favourite"),
        None,
        Some(&token),
        &cfg.pica_channel,
    )?;
    let action = data.get("action").and_then(Value::as_str).unwrap_or("");
    Ok(action == "favourite")
}

pub fn browse_online_comics(
    app: &AppHandle,
    source: &str,
    mode: Option<String>,
    query: Option<String>,
    category: Option<String>,
    sort: Option<String>,
    page: Option<i64>,
) -> Result<OnlineComicPageResult, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let mut cfg = load_stored_config(&conn);
    match source {
        "pica" => browse_pica(&conn, &mut cfg, mode.as_deref(), query.as_deref(), category.as_deref(), sort.as_deref(), page.unwrap_or(1)),
        _ => browse_jm(&cfg, mode.as_deref(), query.as_deref(), category.as_deref(), sort.as_deref(), page.unwrap_or(1)),
    }
}

pub fn get_online_comic_detail(app: &AppHandle, source: &str, comic_id: &str) -> Result<OnlineComicDetail, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let mut cfg = load_stored_config(&conn);
    match source {
        "pica" => get_pica_detail(&conn, &mut cfg, comic_id),
        _ => get_jm_detail(&cfg, comic_id),
    }
}

pub fn get_online_comic_chapter_pages(
    app: &AppHandle,
    source: &str,
    comic_id: &str,
    chapter_id: &str,
) -> Result<OnlineChapterPagesResult, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_config_schema(&conn)?;
    let mut cfg = load_stored_config(&conn);
    match source {
        "pica" => get_pica_chapter_pages(&conn, &mut cfg, comic_id, chapter_id),
        _ => get_jm_chapter_pages(&cfg, comic_id, chapter_id),
    }
}

// ============================================================================
// Image Proxy + JMComic Automatic Unscramble (ffmpeg crop+vstack + Disk Cache)
// ============================================================================

fn jm_scramble_num(photo_id: i64, img_name: &str) -> usize {
    if photo_id < 220980 || img_name.to_lowercase().ends_with(".gif") {
        return 0;
    }
    if photo_id < 268850 {
        return 10;
    }
    let stem = img_name.rsplit_once('.').map(|(s, _)| s).unwrap_or(img_name);
    let h = md5_hex(format!("{photo_id}{stem}").as_bytes());
    let last_byte = *h.as_bytes().last().unwrap_or(&0) as usize;
    if photo_id < 421926 {
        (last_byte % 10) * 2 + 2
    } else {
        (last_byte % 8) * 2 + 2
    }
}

async fn unscramble_with_ffmpeg(raw_bytes: &[u8], num: usize, cache_path: &PathBuf) -> Result<Vec<u8>, String> {
    let tmp_in = cache_path.with_extension("in.tmp");
    let tmp_out = cache_path.with_extension("out.jpg");
    tokio::fs::write(&tmp_in, raw_bytes).await.map_err(|e| e.to_string())?;

    // 1. Probe width & height with ffprobe
    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=s=x:p=0",
        ])
        .arg(&tmp_in)
        .output()
        .await
        .map_err(|e| format!("ffprobe 失败：{e}"))?;

    let dim_str = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    let (w, h) = dim_str
        .split_once('x')
        .and_then(|(ws, hs)| Some((ws.trim().parse::<usize>().ok()?, hs.trim().parse::<usize>().ok()?)))
        .ok_or_else(|| format!("无法解析图片尺寸: {dim_str}"))?;

    if h < num * 2 || num < 2 {
        let _ = tokio::fs::remove_file(&tmp_in).await;
        return Ok(raw_bytes.to_vec());
    }

    // 2. Build reverse-slice crop + vstack filter_complex
    let rem = h % num;
    let slice_h = h / num;
    let mut filter = String::new();
    let mut labels = String::new();
    for i in 0..num {
        let src_idx = num - 1 - i;
        let cur_h = if i == 0 { slice_h + rem } else { slice_h };
        let src_y = if src_idx == num - 1 {
            h - cur_h
        } else {
            src_idx * slice_h
        };
        filter.push_str(&format!("[0:v]crop={w}:{cur_h}:0:{src_y}[s{i}];"));
        labels.push_str(&format!("[s{i}]"));
    }
    filter.push_str(&format!("{labels}vstack=inputs={num}"));

    let status = tokio::process::Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-i"])
        .arg(&tmp_in)
        .args(["-filter_complex", &filter, "-q:v", "3"])
        .arg(&tmp_out)
        .status()
        .await
        .map_err(|e| format!("ffmpeg 反混淆失败：{e}"))?;

    let _ = tokio::fs::remove_file(&tmp_in).await;
    if !status.success() {
        let _ = tokio::fs::remove_file(&tmp_out).await;
        return Ok(raw_bytes.to_vec());
    }
    let out_bytes = tokio::fs::read(&tmp_out).await.map_err(|e| e.to_string())?;
    let _ = tokio::fs::rename(&tmp_out, cache_path).await;
    Ok(out_bytes)
}

pub async fn fetch_online_comic_image(
    app: &AppHandle,
    params: HashMap<String, String>,
) -> Result<(Vec<u8>, String), String> {
    let source = params.get("source").map(String::as_str).unwrap_or("");
    let cfg = {
        let conn = four_domains::open_v2(app)?;
        let _ = ensure_config_schema(&conn);
        load_stored_config(&conn)
    };
    let client = http_client()?;

    match source {
        "jm_cover" => {
            let album_id = params.get("albumId").map(String::as_str).unwrap_or("");
            if album_id.is_empty() || !album_id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err("无效 albumId".to_string());
            }
            let cpath = cache_dir().join(format!("jm_cover_{album_id}.jpg"));
            if let Ok(cached) = tokio::fs::read(&cpath).await {
                if !cached.is_empty() {
                    return Ok((cached, "image/jpeg".to_string()));
                }
            }
            for dom in &cfg.jm_image_domains {
                let url = format!("https://{dom}/media/albums/{album_id}_3x4.jpg");
                if let Ok(resp) = client
                    .get(&url)
                    .header("User-Agent", "Mozilla/5.0 (Linux; Android 10; K)")
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Ok(bytes) = resp.bytes().await {
                            let v = bytes.to_vec();
                            let _ = tokio::fs::write(&cpath, &v).await;
                            return Ok((v, "image/jpeg".to_string()));
                        }
                    }
                }
            }
            Err("拉取 JM 封面失败".to_string())
        }
        "jm" => {
            let photo_id = params.get("photoId").map(String::as_str).unwrap_or("");
            let img = params.get("img").map(String::as_str).unwrap_or("");
            if photo_id.is_empty() || img.is_empty() || img.contains("..") || img.contains('/') {
                return Err("无效 JM 图片参数".to_string());
            }
            let safe_key = md5_hex(format!("jm_{photo_id}_{img}").as_bytes());
            let cpath = cache_dir().join(format!("jm_{safe_key}.jpg"));
            if let Ok(cached) = tokio::fs::read(&cpath).await {
                if !cached.is_empty() {
                    return Ok((cached, "image/jpeg".to_string()));
                }
            }
            let mut raw_opt = None;
            let mut mime = "image/jpeg".to_string();
            for dom in &cfg.jm_image_domains {
                let url = format!("https://{dom}/media/photos/{photo_id}/{img}");
                if let Ok(resp) = client
                    .get(&url)
                    .header("User-Agent", "Mozilla/5.0 (Linux; Android 10; K)")
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
                            mime = ct.to_string();
                        }
                        if let Ok(bytes) = resp.bytes().await {
                            raw_opt = Some(bytes.to_vec());
                            break;
                        }
                    }
                }
            }
            let raw = raw_opt.ok_or_else(|| "拉取 JM 章节图片失败".to_string())?;
            let pid_num = photo_id.parse::<i64>().unwrap_or(0);
            let num = jm_scramble_num(pid_num, img);
            if num >= 2 {
                let unscrambled = unscramble_with_ffmpeg(&raw, num, &cpath).await?;
                Ok((unscrambled, "image/jpeg".to_string()))
            } else {
                let _ = tokio::fs::write(&cpath, &raw).await;
                Ok((raw, mime))
            }
        }
        "pica" => {
            let url = params.get("url").map(String::as_str).unwrap_or("");
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err("无效哔咔图片 URL".to_string());
            }
            let key = md5_hex(url.as_bytes());
            let cpath = cache_dir().join(format!("pica_{key}.bin"));
            if let Ok(cached) = tokio::fs::read(&cpath).await {
                if !cached.is_empty() {
                    let mime = if url.ends_with(".png") { "image/png" } else if url.ends_with(".webp") { "image/webp" } else { "image/jpeg" };
                    return Ok((cached, mime.to_string()));
                }
            }
            let resp = client
                .get(url)
                .header("User-Agent", "okhttp/3.8.1")
                .send()
                .await
                .map_err(|e| format!("拉取哔咔图片失败：{e}"))?;
            if !resp.status().is_success() {
                return Err(format!("哔咔图片 HTTP {}", resp.status()));
            }
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_string();
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
            let _ = tokio::fs::write(&cpath, &bytes).await;
            Ok((bytes, mime))
        }
        _ => Err("不支持的在线漫画源".to_string()),
    }
}

// ============================================================================
// Comic Magnet Search (Sukebei Nyaa + Nyaa RSS)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicMagnetItem {
    pub title: String,
    pub magnet: String,
    pub info_hash: String,
    pub size: String,
    pub seeders: i64,
    pub leechers: i64,
    pub pub_date: String,
    pub source: String,
}

async fn fetch_nyaa_rss(client: &reqwest::Client, url: &str, source_label: &str) -> Vec<ComicMagnetItem> {
    let Ok(resp) = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
    else {
        return Vec::new();
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(xml) = resp.text().await else {
        return Vec::new();
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let mut title = String::new();
        let mut info_hash = String::new();
        let mut size = String::new();
        let mut seeders: i64 = 0;
        let mut leechers: i64 = 0;
        let mut pub_date = String::new();
        for child in node.children().filter(|c| c.is_element()) {
            let tag = child.tag_name().name();
            let text = child.text().unwrap_or("").trim();
            match tag {
                "title" => title = text.to_string(),
                "infoHash" => info_hash = text.to_lowercase(),
                "size" => size = text.to_string(),
                "seeders" => seeders = text.parse().unwrap_or(0),
                "leechers" => leechers = text.parse().unwrap_or(0),
                "pubDate" => pub_date = text.to_string(),
                _ => {}
            }
        }
        if title.is_empty() || info_hash.is_empty() {
            continue;
        }
        let magnet = format!(
            "magnet:?xt=urn:btih:{info_hash}&dn={}&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce",
            urlencoding(&title)
        );
        items.push(ComicMagnetItem {
            title,
            magnet,
            info_hash,
            size,
            seeders,
            leechers,
            pub_date,
            source: source_label.to_string(),
        });
    }
    items
}

pub fn search_comic_magnets(query: &str) -> Result<Vec<ComicMagnetItem>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let fut = async move {
        let client = http_client()?;
        let eq = urlencoding(q);
        let sukebei_manga_url = format!("https://sukebei.nyaa.si/?page=rss&q={eq}&c=1_4&s=seeders&o=desc");
        let sukebei_all_url = format!("https://sukebei.nyaa.si/?page=rss&q={eq}&c=0_0&s=seeders&o=desc");
        let nyaa_manga_url = format!("https://nyaa.si/?page=rss&q={eq}&c=3_0&s=seeders&o=desc");

        let (mut r1, r3) = tokio::join!(
            fetch_nyaa_rss(&client, &sukebei_manga_url, "Sukebei"),
            fetch_nyaa_rss(&client, &nyaa_manga_url, "Nyaa")
        );
        if r1.len() < 3 {
            let extra = fetch_nyaa_rss(&client, &sukebei_all_url, "Sukebei");
            r1.extend(extra.await);
        }
        let mut seen = std::collections::HashSet::new();
        let mut combined = Vec::new();
        for item in r1.into_iter().chain(r3.into_iter()) {
            if seen.insert(item.info_hash.clone()) {
                combined.push(item);
            }
        }
        combined.sort_by(|a, b| b.seeders.cmp(&a.seeders));
        combined.truncate(30);
        Ok::<Vec<ComicMagnetItem>, String>(combined)
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(fut)
    }
}

// ============================================================================
// One-Click .cbz Packaging & Download Task
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CbzPackageTask {
    pub task_id: String,
    pub status: String, // "running" | "done" | "error"
    pub title: String,
    pub total: usize,
    pub completed: usize,
    pub file_name: Option<String>,
    pub download_url: Option<String>,
    pub size_bytes: Option<u64>,
    pub error: Option<String>,
}

fn cbz_tasks_store() -> &'static Mutex<HashMap<String, CbzPackageTask>> {
    static STORE: OnceLock<Mutex<HashMap<String, CbzPackageTask>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sanitize_cbz_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    let truncated: String = trimmed.chars().take(80).collect();
    if truncated.is_empty() {
        "comic".to_string()
    } else {
        truncated
    }
}

pub fn get_online_comic_cbz_status(task_id: &str) -> Result<CbzPackageTask, String> {
    let map = cbz_tasks_store().lock().map_err(|_| "锁异常".to_string())?;
    map.get(task_id)
        .cloned()
        .ok_or_else(|| format!("未找到打包任务：{task_id}"))
}

pub fn start_online_comic_cbz(
    app: &AppHandle,
    source: &str,
    comic_id: &str,
    chapter_id: &str,
    title: &str,
    author: Option<&str>,
) -> Result<CbzPackageTask, String> {
    let pages = get_online_comic_chapter_pages(app, source, comic_id, chapter_id)?;
    if pages.images.is_empty() {
        return Err("该章节暂无图片可打包".to_string());
    }

    let task_id = format!("cbz-{}", md5_hex(format!("{source}:{comic_id}:{chapter_id}:{}", now_secs()).as_bytes())[..12].to_string());
    let base_name = if let Some(a) = author.map(str::trim).filter(|s| !s.is_empty()) {
        format!("[{}] {}", sanitize_cbz_filename(a), sanitize_cbz_filename(title))
    } else {
        sanitize_cbz_filename(title)
    };
    let file_name = format!("{base_name}.cbz");

    let initial = CbzPackageTask {
        task_id: task_id.clone(),
        status: "running".to_string(),
        title: title.to_string(),
        total: pages.images.len(),
        completed: 0,
        file_name: Some(file_name.clone()),
        download_url: None,
        size_bytes: None,
        error: None,
    };
    {
        let mut map = cbz_tasks_store().lock().map_err(|_| "锁异常".to_string())?;
        map.insert(task_id.clone(), initial.clone());
    }

    let app_clone = app.clone();
    let images = pages.images;
    let tid = task_id.clone();
    crate::rt::spawn(async move {
        let download_dir = PathBuf::from(
            std::env::var("PML_DOWNLOAD_DIR").unwrap_or_else(|_| "/data/downloads".to_string()),
        );
        let _ = tokio::fs::create_dir_all(&download_dir).await;
        let out_path = download_dir.join(&file_name);

        let mut downloaded_pages: Vec<(String, Vec<u8>)> = Vec::with_capacity(images.len());
        for (idx, rel_url) in images.iter().enumerate() {
            let mut query_map = HashMap::new();
            if let Some((_, qs)) = rel_url.split_once('?') {
                for (k, v) in url::form_urlencoded::parse(qs.as_bytes()) {
                    query_map.insert(k.into_owned(), v.into_owned());
                }
            }
            match fetch_online_comic_image(&app_clone, query_map).await {
                Ok((bytes, mime)) => {
                    let ext = if mime.contains("png") {
                        "png"
                    } else if mime.contains("webp") {
                        "webp"
                    } else if mime.contains("gif") {
                        "gif"
                    } else {
                        "jpg"
                    };
                    downloaded_pages.push((format!("{:04}.{ext}", idx + 1), bytes));
                }
                Err(_) => {
                    // skip broken page rather than aborting entire chapter
                }
            }
            if let Ok(mut map) = cbz_tasks_store().lock() {
                if let Some(t) = map.get_mut(&tid) {
                    t.completed = idx + 1;
                }
            }
        }

        if downloaded_pages.is_empty() {
            if let Ok(mut map) = cbz_tasks_store().lock() {
                if let Some(t) = map.get_mut(&tid) {
                    t.status = "error".to_string();
                    t.error = Some("所有图片拉取失败，无法生成 CBZ".to_string());
                }
            }
            return;
        }

        let write_res = tokio::task::spawn_blocking(move || -> Result<u64, String> {
            let file = std::fs::File::create(&out_path).map_err(|e| format!("创建 CBZ 失败：{e}"))?;
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in downloaded_pages {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                zip.write_all(&data).map_err(|e| e.to_string())?;
            }
            zip.finish().map_err(|e| e.to_string())?;
            let meta = std::fs::metadata(&out_path).map_err(|e| e.to_string())?;
            Ok(meta.len())
        })
        .await;

        if let Ok(mut map) = cbz_tasks_store().lock() {
            if let Some(t) = map.get_mut(&tid) {
                match write_res {
                    Ok(Ok(sz)) => {
                        t.status = "done".to_string();
                        t.size_bytes = Some(sz);
                        t.download_url = Some(format!("/downloads/{}", urlencoding(&file_name)));
                    }
                    Ok(Err(err)) => {
                        t.status = "error".to_string();
                        t.error = Some(err);
                    }
                    Err(err) => {
                        t.status = "error".to_string();
                        t.error = Some(err.to_string());
                    }
                }
            }
        }
    });

    Ok(initial)
}
