//! keygen —— 开发者离线工具：生成密钥对、签发授权码。
//!
//! ⚠️ 本工具含私钥，**绝不分发给客户**。
//!
//! 用法：
//!   keygen gen-keys                          生成密钥对（私钥写入 private_key.bin，打印公钥）
//!   keygen sign <machine> [days] [customer]  用私钥签发授权码
//!     - machine:   客户提供的机器码（XXXX-XXXX-XXXX-XXXX）
//!     - days:      授权天数；0 = 永久（默认 0）
//!     - customer:  客户名（可选）

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rand::RngCore;
use std::fs;
use std::time::UNIX_EPOCH;

const PRIV_FILE: &str = "private_key.bin";
const PRODUCT: &str = "umya-excel";
const VERSION: u32 = 1;
const EDITION: &str = "pro";
const EXPIRES_NEVER: u64 = 0;

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn today_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86400
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    match mode {
        "gen-keys" => gen_keys(),
        "sign" => sign(&args),
        _ => {
            eprintln!("usage:");
            eprintln!("  keygen gen-keys");
            eprintln!("  keygen sign <machine-code> [days(0=forever)] [customer]");
            std::process::exit(1);
        }
    }
}

fn gen_keys() {
    // 随机 32 字节种子 → 私钥；用 from_bytes 避免 rand_core 特性依赖
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let verifying = signing.verifying_key();
    let pk = verifying.to_bytes();

    fs::write(PRIV_FILE, seed).expect("write private key");

    // 打印可直接粘贴进 src/license/crypto.rs 的公钥数组
    let mut s = String::from("pub const DEVELOPER_PUBLIC_KEY: [u8; 32] = [\n    ");
    for (i, byte) in pk.iter().enumerate() {
        s.push_str(&format!("0x{:02x}, ", byte));
        if i % 8 == 7 {
            s.push_str("\n    ");
        }
    }
    s.push_str("\n];");

    println!("✅ 私钥已写入 {}（务必保密、勿提交 git）", PRIV_FILE);
    println!("\n👇 把下面这段粘贴到 src/license/crypto.rs：\n\n{}\n", s);
}

fn sign(args: &[String]) {
    let machine = args.get(2).expect("missing <machine-code>");
    let days: u64 = args
        .get(3)
        .and_then(|d| d.parse().ok())
        .unwrap_or(0); // 0 = 永久
    let customer = args.get(4).cloned().unwrap_or_default();

    let seed_bytes = fs::read(PRIV_FILE).expect("read private key failed (run `gen-keys` first)");
    if seed_bytes.len() != 32 {
        panic!("private key file corrupt (expected 32 bytes)");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);
    let signing = SigningKey::from_bytes(&seed);

    let today = today_day();
    let expires = if days == 0 { EXPIRES_NEVER } else { today + days };

    // ⚠️ 必须与 src/license/payload.rs::LicensePayload::to_text 格式完全一致
    let text = format!(
        "v={}\np={}\nm={}\ni={}\ne={}\ned={}\nc={}\n",
        VERSION, PRODUCT, machine, today, expires, EDITION, customer
    );
    let sig = signing.sign(text.as_bytes()).to_bytes();
    let license = format!("{}.{}", b64(text.as_bytes()), b64(&sig));

    println!("{}", license);
}
