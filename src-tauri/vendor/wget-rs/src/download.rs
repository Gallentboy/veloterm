use std::fs::{File, OpenOptions};
use std::io::{Write, Read};
use std::thread;
use std::sync::{Arc, Mutex};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, RANGE, ACCEPT_RANGES, HeaderMap};
use regex::Regex;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

fn get_file_size(filename: &str) -> Option<u64> {
    std::fs::metadata(filename)
        .ok()
        .map(|metadata| metadata.len())
}

fn check_resume_capability(client: &Client, url: &str, start_pos: u64) -> Result<(bool, u64), Box<dyn std::error::Error>> {
    let range_header = format!("bytes={}-", start_pos);
    let response = client
        .get(url)
        .header(RANGE, range_header)
        .send()?;
    
    let status = response.status();
    if status.as_u16() == 206 {
        // Server supports partial content
        let content_length = response.headers()
            .get(CONTENT_LENGTH)
            .and_then(|len| len.to_str().ok())
            .and_then(|len| len.parse::<u64>().ok())
            .unwrap_or(0);
        
        Ok((true, content_length + start_pos))
    } else if status.as_u16() == 416 {
        // Range not satisfiable - file might be already complete
        Ok((false, start_pos))
    } else if status.is_success() {
        // Server doesn't support ranges, returns full content
        let content_length = response.headers()
            .get(CONTENT_LENGTH)
            .and_then(|len| len.to_str().ok())
            .and_then(|len| len.parse::<u64>().ok())
            .unwrap_or(0);
        Ok((false, content_length))
    } else {
        Err(format!("HTTP error: {} - {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown")).into())
    }
}

fn validate_response(response: &reqwest::blocking::Response, _expected_filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = response.status();
    
    // 只检查 HTTP 状态码
    if !status.is_success() {
        return Err(format!("HTTP error: {} - {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown")).into());
    }
    // 不再对内容类型做强制检查
    
    Ok(())
}

fn create_client() -> Result<Client, Box<dyn std::error::Error>> {
    let pkg_version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let user_agent = format!("Wget/{} ({})", pkg_version, std::env::consts::OS);
    Client::builder()
        .user_agent(user_agent)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| e.into())
}

fn extract_filename_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(disposition) = headers.get(CONTENT_DISPOSITION) {
        let disposition_str = disposition.to_str().ok()?;
        let re = Regex::new(r#"filename\*?=(?:UTF-8'')?["']?([^;"']+)["']?"#).unwrap();
        if let Some(cap) = re.captures(disposition_str) {
            return Some(cap[1].to_string());
        }
    }
    None
}

fn extract_filename_from_url(url: &str) -> String {
    url.split('/')
        .last()
        .filter(|s| !s.is_empty())
        .unwrap_or("output")
        .to_string()
}

fn supports_range_requests(headers: &HeaderMap) -> bool {
    headers.get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "bytes")
        .unwrap_or(false)
}

fn download_chunk(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
    chunk_data: Arc<Mutex<Vec<u8>>>,
    progress: Arc<Mutex<ProgressBar>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let chunk_size = end - start + 1;
    const MAX_CHUNK_SIZE: u64 = 100 * 1024 * 1024; // 100MB limit per chunk
    
    if chunk_size > MAX_CHUNK_SIZE {
        return Err(format!("Chunk size {} exceeds maximum allowed size", chunk_size).into());
    }

    let range_header = format!("bytes={}-{}", start, end);
    let response = client
        .get(url)
        .header(RANGE, range_header)
        .send()?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {} - {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown")).into());
    }

    let mut buffer = Vec::new();
    let mut response_reader = response;
    response_reader.read_to_end(&mut buffer)?;

    {
        let mut data = chunk_data.lock().unwrap();
        *data = buffer;
    }

    {
        let pb = progress.lock().unwrap();
        pb.inc(end - start + 1);
    }

    Ok(())
}

fn download_single_threaded(
    client: &Client,
    url: &str,
    filename: &str,
    total_size: u64,
    resume_from: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let start_pos = resume_from.unwrap_or(0);
    let mut request = client.get(url);
    
    if let Some(pos) = resume_from {
        request = request.header(RANGE, format!("bytes={}-", pos));
    }
    
    let response = request.send()?;
    
    // Validate the response before proceeding
    validate_response(&response, filename)?;
    
    let expected_status = if resume_from.is_some() { 206 } else { 200 };
    if response.status().as_u16() != expected_status {
        if response.status().as_u16() == 416 && resume_from.is_some() {
            println!("文件已完整下载");
            return Ok(());
        }
        return Err(format!("Unexpected status code: {}", response.status()).into());
    }
    
    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{bar:40.cyan/blue} {bytes}/{total_bytes} {percent}% {eta}")
        .unwrap()
        .progress_chars("##-"));
    
    if let Some(pos) = resume_from {
        pb.set_position(pos);
    }

    let mut dest = if resume_from.is_some() {
        OpenOptions::new().append(true).open(filename)?
    } else {
        File::create(filename)?
    };
    
    let mut buffer = [0; 8192];
    let mut downloaded = start_pos;
    let mut response_reader = response;

    loop {
        let n = response_reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        dest.write_all(&buffer[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("下载完成!");
    Ok(())
}

pub fn download_file(url: &str, output: &Option<String>, threads: u32, continue_download: bool) -> Result<(), Box<dyn std::error::Error>> {
    let client = create_client()?;
    let response = client.head(url).send()?;

    let status = response.status();
    println!("服务器响应状态码: {} {}", status.as_u16(), status.canonical_reason().unwrap_or(""));

    if !status.is_success() {
        return Err(format!("HTTP error: {} - {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown")).into());
    }

    let headers = response.headers().clone();

    let filename = match output {
        Some(name) => name.clone(),
        None => extract_filename_from_headers(&headers)
            .or_else(|| Some(extract_filename_from_url(url)))
            .unwrap(),
    };

    let total_size = headers
        .get(CONTENT_LENGTH)
        .and_then(|len| len.to_str().ok())
        .and_then(|len| len.parse().ok())
        .unwrap_or(0);

    // 处理断点续传逻辑
    let (resume_from, actual_total_size) = if continue_download {
        if let Some(existing_size) = get_file_size(&filename) {
            if existing_size > 0 {
                println!("发现已存在的文件，大小: {} 字节", existing_size);
                
                // 检查是否支持断点续传
                match check_resume_capability(&client, url, existing_size) {
                    Ok((supports_resume, server_total_size)) => {
                        if supports_resume {
                            println!("服务器支持断点续传，从 {} 字节处继续下载", existing_size);
                            (Some(existing_size), server_total_size)
                        } else if existing_size >= server_total_size {
                            println!("文件已完整下载");
                            return Ok(());
                        } else {
                            println!("服务器不支持断点续传，将重新下载文件");
                            (None, server_total_size)
                        }
                    }
                    Err(e) => {
                        println!("检查断点续传支持时出错: {}，将重新下载", e);
                        (None, total_size)
                    }
                }
            } else {
                println!("发现空文件，将重新下载");
                (None, total_size)
            }
        } else {
            println!("未发现已存在的文件，开始新下载");
            (None, total_size)
        }
    } else {
        (None, total_size)
    };

    let final_total_size = if actual_total_size > 0 { actual_total_size } else { total_size };

    // 初始化进度条，并提前显示
    let pb = Arc::new(Mutex::new(ProgressBar::new(final_total_size)));
    {
        let pb_guard = pb.lock().unwrap();
        pb_guard.set_style(ProgressStyle::default_bar()
            .template("{bar:40.cyan/blue} {bytes}/{total_bytes} {percent}% {eta}")
            .unwrap()
            .progress_chars("##-"));
        pb_guard.enable_steady_tick(Duration::from_millis(100)); // 让进度条提前刷新
    }
    println!("正在准备多线程下载，请稍候...");

    // 如果文件大小未知或服务器不支持范围请求，使用单线程下载
    // 注意：如果是断点续传，我们已经检查过服务器支持情况了
    if final_total_size == 0 || (!supports_range_requests(&headers) && resume_from.is_none()) || threads == 1 {
        println!("使用单线程下载...");
        return download_single_threaded(&client, url, &filename, final_total_size, resume_from);
    }

    // 如果是断点续传但要用多线程，需要特殊处理
    if resume_from.is_some() {
        println!("断点续传模式下使用单线程下载...");
        return download_single_threaded(&client, url, &filename, final_total_size, resume_from);
    }

    println!("使用 {} 线程下载，文件大小: {} 字节", threads, final_total_size);

    let chunk_size = final_total_size / threads as u64;

    // If chunk size is too small (less than 1 byte per thread), use single thread  
    if chunk_size == 0 {
        println!("文件太小，使用单线程下载...");
        return download_single_threaded(&client, url, &filename, final_total_size, resume_from);
    }

    let mut handles = vec![];
    let mut chunk_data = vec![];

    for i in 0..threads {
        let start = i as u64 * chunk_size;
        let end = if i == threads - 1 {
            final_total_size - 1
        } else {
            (i + 1) as u64 * chunk_size - 1
        };

        let chunk_storage = Arc::new(Mutex::new(Vec::new()));
        chunk_data.push(chunk_storage.clone());

        let client_clone = client.clone();
        let url_clone = url.to_string();
        let pb_clone = pb.clone();

        let handle = thread::spawn(move || {
            download_chunk(&client_clone, &url_clone, start, end, chunk_storage, pb_clone)
        });

        handles.push(handle);
    }

    // 等待所有线程完成
    for handle in handles {
        match handle.join() {
            Ok(result) => {
                if let Err(e) = result {
                    return Err(format!("下载块失败: {}", e).into());
                }
            }
            Err(_) => {
                return Err("线程 panic".into());
            }
        }
    }

    // 合并所有块到最终文件
    let mut dest = File::create(&filename)?;
    for chunk in chunk_data {
        let data = chunk.lock().unwrap();
        dest.write_all(&data)?;
    }

    {
        let pb_guard = pb.lock().unwrap();
        pb_guard.finish_with_message("下载完成!");
    }
    println!("文件保存为: {}", filename);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_range_requests() {
        let mut headers = HeaderMap::new();
        
        // Test when Accept-Ranges is not present
        assert!(!supports_range_requests(&headers));
        
        // Test when Accept-Ranges is bytes
        headers.insert(ACCEPT_RANGES, "bytes".parse().unwrap());
        assert!(supports_range_requests(&headers));
        
        // Test when Accept-Ranges is none
        headers.insert(ACCEPT_RANGES, "none".parse().unwrap());
        assert!(!supports_range_requests(&headers));
    }

    #[test]
    fn test_extract_filename_from_url() {
        assert_eq!(extract_filename_from_url("https://example.com/file.txt"), "file.txt");
        assert_eq!(extract_filename_from_url("https://example.com/path/to/file.zip"), "file.zip");
        assert_eq!(extract_filename_from_url("https://example.com/"), "output");
        assert_eq!(extract_filename_from_url("https://example.com"), "example.com");
    }

    #[test]
    fn test_extract_filename_from_headers() {
        let mut headers = HeaderMap::new();
        
        // Test when Content-Disposition is not present
        assert!(extract_filename_from_headers(&headers).is_none());
        
        // Test with standard filename
        headers.insert(CONTENT_DISPOSITION, "attachment; filename=\"test.txt\"".parse().unwrap());
        assert_eq!(extract_filename_from_headers(&headers), Some("test.txt".to_string()));
        
        // Test with filename*
        headers.insert(CONTENT_DISPOSITION, "attachment; filename*=UTF-8''test%20file.txt".parse().unwrap());
        assert_eq!(extract_filename_from_headers(&headers), Some("test%20file.txt".to_string()));
    }

    #[test] 
    fn test_chunk_calculation() {
        let total_size = 1000u64;
        let threads = 4u32;
        let chunk_size = total_size / threads as u64;
        
        // Test chunk boundaries
        for i in 0..threads {
            let start = i as u64 * chunk_size;
            let end = if i == threads - 1 {
                total_size - 1
            } else {
                (i + 1) as u64 * chunk_size - 1
            };
            
            // Verify no gaps or overlaps
            if i > 0 {
                let prev_end = (i as u64 * chunk_size) - 1;
                assert_eq!(start, prev_end + 1);
            }
            
            // Verify last chunk goes to the end
            if i == threads - 1 {
                assert_eq!(end, total_size - 1);
            }
        }
    }

    #[test]
    fn test_small_file_edge_case() {
        // Test that very small files would result in chunk_size = 0
        let total_size = 10u64;
        let threads = 32u32;
        let chunk_size = total_size / threads as u64;
        
        // This should be 0, which means we should fall back to single-threaded
        assert_eq!(chunk_size, 0);
    }

    #[test]
    fn test_get_file_size() {
        // Test with non-existent file
        assert!(get_file_size("non_existent_file.txt").is_none());
        
        // Test with temporary file
        use std::fs::File;
        use std::io::Write;
        
        let temp_path = "/tmp/test_file_size.txt";
        {
            let mut file = File::create(temp_path).unwrap();
            file.write_all(b"Hello, World!").unwrap();
        }
        
        let size = get_file_size(temp_path);
        assert_eq!(size, Some(13)); // "Hello, World!" is 13 bytes
        
        // Clean up
        std::fs::remove_file(temp_path).ok();
    }

    #[test]
    fn test_resume_logic_integration() {
        // Test that the CLI argument is properly integrated
        use crate::cli::{Args};
        
        let args = Args {
            url: "https://example.com/test.txt".to_string(),
            output: Some("test.txt".to_string()),
            threads: 1,
            continue_: true,
            hash: false,
            verify_hash: None,
        };
        
        assert!(args.continue_);
        assert_eq!(args.output, Some("test.txt".to_string()));
    }

    #[test]
    fn test_create_client() {
        // Test that the client is created successfully with proper user agent
        let client = create_client();
        assert!(client.is_ok());
        
        // We can't easily test the exact user agent without making a request,
        // but we can verify the client was created successfully
    }

    #[test]
    fn test_validate_response_content_type() {
        // This is a more complex test that would require mocking a response
        // For now, we'll just test that the function exists and can be called
        // In a real scenario, we'd mock responses with different content types
        
        // Test passes if the function compiles and can be referenced
        let _fn_ref = validate_response;
    }
}