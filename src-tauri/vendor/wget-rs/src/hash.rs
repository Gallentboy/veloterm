use std::fs::File;
use std::io::{BufReader, Read};
use std::fmt;
use sha2::{Sha256, Digest};
use sha1::Sha1;
use md5::Md5;
use crc32fast::Hasher as Crc32Hasher;

/// 支持的哈希算法类型
#[derive(Debug, Clone, PartialEq)]
pub enum HashType {
    MD5,
    SHA1,
    SHA256,
    CRC32,
}

impl fmt::Display for HashType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HashType::MD5 => write!(f, "MD5"),
            HashType::SHA1 => write!(f, "SHA1"),
            HashType::SHA256 => write!(f, "SHA256"),
            HashType::CRC32 => write!(f, "CRC32"),
        }
    }
}

impl HashType {
    /// 从字符串解析哈希类型
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<HashType> {
        match s.to_lowercase().as_str() {
            "md5" => Some(HashType::MD5),
            "sha1" => Some(HashType::SHA1),
            "sha256" => Some(HashType::SHA256),
            "crc32" => Some(HashType::CRC32),
            _ => None,
        }
    }

    /// 获取所有支持的哈希类型
    pub fn all() -> Vec<HashType> {
        vec![HashType::MD5, HashType::SHA1, HashType::SHA256, HashType::CRC32]
    }
}

/// 哈希计算结果
#[derive(Debug, Clone)]
pub struct HashResult {
    pub hash_type: HashType,
    pub value: String,
}

impl fmt::Display for HashResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.hash_type, self.value)
    }
}

/// 计算文件的指定哈希值
pub fn calculate_hash(file_path: &str, hash_type: &HashType) -> Result<HashResult, Box<dyn std::error::Error>> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = [0; 8192];

    let hash_value = match hash_type {
        HashType::MD5 => {
            let mut hasher = Md5::new();
            loop {
                let bytes_read = reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                hasher.update(&buffer[..bytes_read]);
            }
            format!("{:x}", hasher.finalize())
        }
        HashType::SHA1 => {
            let mut hasher = Sha1::new();
            loop {
                let bytes_read = reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                hasher.update(&buffer[..bytes_read]);
            }
            format!("{:x}", hasher.finalize())
        }
        HashType::SHA256 => {
            let mut hasher = Sha256::new();
            loop {
                let bytes_read = reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                hasher.update(&buffer[..bytes_read]);
            }
            format!("{:x}", hasher.finalize())
        }
        HashType::CRC32 => {
            let mut hasher = Crc32Hasher::new();
            loop {
                let bytes_read = reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                hasher.update(&buffer[..bytes_read]);
            }
            format!("{:08x}", hasher.finalize())
        }
    };

    Ok(HashResult {
        hash_type: hash_type.clone(),
        value: hash_value,
    })
}

/// 计算文件的所有支持的哈希值
pub fn calculate_all_hashes(file_path: &str) -> Result<Vec<HashResult>, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    
    for hash_type in HashType::all() {
        match calculate_hash(file_path, &hash_type) {
            Ok(result) => results.push(result),
            Err(e) => return Err(format!("计算 {} 哈希失败: {}", hash_type, e).into()),
        }
    }
    
    Ok(results)
}

/// 验证文件哈希值
#[allow(dead_code)]
pub fn verify_hash(file_path: &str, expected_hash: &str, hash_type: &HashType) -> Result<bool, Box<dyn std::error::Error>> {
    let calculated = calculate_hash(file_path, hash_type)?;
    Ok(calculated.value.to_lowercase() == expected_hash.to_lowercase())
}

/// 自动检测哈希类型（基于哈希值长度）
pub fn detect_hash_type(hash_value: &str) -> Option<HashType> {
    match hash_value.len() {
        8 => Some(HashType::CRC32),
        32 => Some(HashType::MD5),
        40 => Some(HashType::SHA1),
        64 => Some(HashType::SHA256),
        _ => None,
    }
}

/// 显示哈希计算结果
pub fn display_hash_results(results: &[HashResult], file_path: &str) {
    println!("\n文件 {} 的哈希值:", file_path);
    for result in results {
        println!("  {}", result);
    }
}

/// 验证并显示哈希比较结果
pub fn verify_and_display(file_path: &str, expected_hash: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 自动检测哈希类型
    let hash_type = detect_hash_type(expected_hash)
        .ok_or_else(|| format!("无法识别哈希值格式: {}", expected_hash))?;
    
    let calculated = calculate_hash(file_path, &hash_type)?;
    let matches = calculated.value.to_lowercase() == expected_hash.to_lowercase();
    
    println!("\n哈希验证结果:");
    println!("  文件: {}", file_path);
    println!("  算法: {}", hash_type);
    println!("  计算值: {}", calculated.value);
    println!("  期望值: {}", expected_hash);
    println!("  结果: {}", if matches { "✓ 匹配" } else { "✗ 不匹配" });
    
    if !matches {
        return Err("哈希验证失败".into());
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn create_test_file(content: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let test_file = format!("/tmp/test_hash_file_{}.txt", timestamp);
        let mut file = File::create(&test_file).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        test_file
    }

    #[test]
    fn test_hash_type_from_str() {
        assert_eq!(HashType::from_str("md5"), Some(HashType::MD5));
        assert_eq!(HashType::from_str("MD5"), Some(HashType::MD5));
        assert_eq!(HashType::from_str("sha1"), Some(HashType::SHA1));
        assert_eq!(HashType::from_str("SHA256"), Some(HashType::SHA256));
        assert_eq!(HashType::from_str("crc32"), Some(HashType::CRC32));
        assert_eq!(HashType::from_str("invalid"), None);
    }

    #[test]
    fn test_detect_hash_type() {
        assert_eq!(detect_hash_type("12345678"), Some(HashType::CRC32));
        assert_eq!(detect_hash_type("5d41402abc4b2a76b9719d911017c592"), Some(HashType::MD5));
        assert_eq!(detect_hash_type("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"), Some(HashType::SHA1));
        assert_eq!(detect_hash_type("e258d248fda94c63753607f7c4494ee0fcbe92f1a76bfdac795c9d84101eb317"), Some(HashType::SHA256));
        assert_eq!(detect_hash_type("invalid"), None);
    }

    #[test]
    fn test_calculate_hash() {
        let test_file = create_test_file("hello");
        
        // Test MD5
        let md5_result = calculate_hash(&test_file, &HashType::MD5).unwrap();
        assert_eq!(md5_result.hash_type, HashType::MD5);
        assert_eq!(md5_result.value, "5d41402abc4b2a76b9719d911017c592");
        
        // Test SHA1
        let sha1_result = calculate_hash(&test_file, &HashType::SHA1).unwrap();
        assert_eq!(sha1_result.hash_type, HashType::SHA1);
        assert_eq!(sha1_result.value, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
        
        // Test SHA256
        let sha256_result = calculate_hash(&test_file, &HashType::SHA256).unwrap();
        assert_eq!(sha256_result.hash_type, HashType::SHA256);
        assert_eq!(sha256_result.value, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        
        // Test CRC32
        let crc32_result = calculate_hash(&test_file, &HashType::CRC32).unwrap();
        assert_eq!(crc32_result.hash_type, HashType::CRC32);
        assert_eq!(crc32_result.value, "3610a686");
        
        // Clean up
        fs::remove_file(test_file).unwrap();
    }

    #[test]
    fn test_verify_hash() {
        let test_file = create_test_file("hello");
        
        // Test valid hash
        assert!(verify_hash(&test_file, "5d41402abc4b2a76b9719d911017c592", &HashType::MD5).unwrap());
        
        // Test invalid hash
        assert!(!verify_hash(&test_file, "invalid_hash", &HashType::MD5).unwrap());
        
        // Test case insensitive
        assert!(verify_hash(&test_file, "5D41402ABC4B2A76B9719D911017C592", &HashType::MD5).unwrap());
        
        // Clean up
        fs::remove_file(test_file).unwrap();
    }

    #[test]
    fn test_calculate_all_hashes() {
        let test_file = create_test_file("hello");
        
        let results = calculate_all_hashes(&test_file).unwrap();
        assert_eq!(results.len(), 4);
        
        // Verify all hash types are present
        let hash_types: Vec<HashType> = results.iter().map(|r| r.hash_type.clone()).collect();
        assert!(hash_types.contains(&HashType::MD5));
        assert!(hash_types.contains(&HashType::SHA1));
        assert!(hash_types.contains(&HashType::SHA256));
        assert!(hash_types.contains(&HashType::CRC32));
        
        // Clean up
        fs::remove_file(test_file).unwrap();
    }
}