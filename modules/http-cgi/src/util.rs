use std::{error::Error, path::PathBuf};
use vibeio::fs;

#[allow(dead_code)]
#[cfg(unix)]
pub async fn get_executable(
    execute_pathbuf: &PathBuf,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(&execute_pathbuf).await?;
    let permissions = metadata.permissions();
    let is_executable = permissions.mode() & 0o111 != 0;

    if !is_executable {
        Err(anyhow::anyhow!("The CGI program is not executable"))?
    }

    let executable_params_vector = vec![execute_pathbuf.to_string_lossy().to_string()];
    Ok(executable_params_vector)
}

#[allow(dead_code)]
#[cfg(not(unix))]
pub async fn get_executable(
    execute_pathbuf: &PathBuf,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
    let magic_signature_buffer = vec![0u8; 2].into_boxed_slice();
    let open_file = fs::File::open(&execute_pathbuf).await?;
    let open_file_result = open_file.read_exact_at(magic_signature_buffer, 0).await;
    if open_file_result.0.is_err() {
        Err(anyhow::anyhow!("Failed to read the CGI program signature"))?
    }

    match bytes::Bytes::from_owner(open_file_result.1).as_ref() {
        b"PE" => {
            // Windows executables
            let executable_params_vector = vec![execute_pathbuf.to_string_lossy().to_string()];
            Ok(executable_params_vector)
        }
        b"#!" => {
            // Scripts with a shebang line
            let mut shebang_line_bytes = Vec::new();
            let mut shebang_bytes_read = 0;
            loop {
                let buf = vec![0u8; 1024].into_boxed_slice();
                let read_result = open_file.read_at(buf, shebang_bytes_read).await;
                let read = read_result.0?;
                let mut buf = bytes::Bytes::from_owner(read_result.1);
                buf.truncate(read);

                shebang_bytes_read += read as u64;
                if let Some(index) = memchr::memchr(b'\n', &buf) {
                    shebang_line_bytes.extend_from_slice(&buf[..index + 1]);
                    break;
                } else if let Some(index) = memchr::memchr(b'\r', &buf) {
                    shebang_line_bytes.extend_from_slice(&buf[..index + 1]);
                    break;
                } else {
                    shebang_line_bytes.extend_from_slice(&buf);
                }
            }
            let shebang_line = String::from_utf8_lossy(&shebang_line_bytes);

            let mut command_begin: Vec<String> = shebang_line[2..]
                .replace("\r", "")
                .replace("\n", "")
                .split(" ")
                .map(|s| s.to_owned())
                .collect();
            command_begin.push(execute_pathbuf.to_string_lossy().to_string());
            Ok(command_begin)
        }
        _ => {
            // It's not executable
            Err(anyhow::anyhow!("The CGI program is not executable"))?
        }
    }
}

pub struct SendWrapBody<B> {
    inner: send_wrapper::SendWrapper<std::pin::Pin<Box<B>>>,
}

impl<B> SendWrapBody<B> {
    #[inline]
    pub fn new(inner: B) -> Self {
        Self {
            inner: send_wrapper::SendWrapper::new(Box::pin(inner)),
        }
    }
}

impl<B> http_body::Body for SendWrapBody<B>
where
    B: http_body::Body + 'static,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.inner.as_mut().poll_frame(cx)
    }
}
