use std::error::Error;

use ferron_core::config::ServerConfigurationBlock;

/// Rotates the log file if it is too large
pub(crate) async fn rotate_log_file(
    log_filename: &str,
    rotate_keep: Option<usize>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // If we are not keeping any logs, just delete the current log file
    if rotate_keep == Some(0) {
        tokio::fs::remove_file(log_filename).await?;
        return Ok(());
    }

    // Find the oldest log file
    let mut oldest_log_file_suffix = 0;
    while rotate_keep.is_none_or(|k| oldest_log_file_suffix < k)
        && tokio::fs::try_exists(format!("{log_filename}.{}", oldest_log_file_suffix + 1)).await?
    {
        oldest_log_file_suffix += 1;
    }

    // Delete the oldest log file if we are keeping too many
    if rotate_keep.is_some_and(|k| oldest_log_file_suffix >= k) {
        tokio::fs::remove_file(format!("{log_filename}.{oldest_log_file_suffix}")).await?;
        oldest_log_file_suffix -= 1;
    }

    // Rotate the log files
    for i in (0..=oldest_log_file_suffix).rev() {
        tokio::fs::rename(
            format!(
                "{log_filename}{}",
                if i == 0 {
                    String::new()
                } else {
                    format!(".{i}")
                }
            ),
            format!("{log_filename}.{}", i + 1),
        )
        .await?;
    }

    Ok(())
}

/// Rotation configuration for a log file
#[derive(Clone, Copy)]
pub(crate) struct RotationConfig {
    /// Rotate when file size exceeds this value (in bytes)
    pub(crate) rotate_size: Option<u64>,
    /// Number of rotated log files to keep
    pub(crate) rotate_keep: Option<usize>,
}

impl RotationConfig {
    /// Read rotation configuration from the log config block
    pub(crate) fn read_from_config(
        log_config: &ServerConfigurationBlock,
        rotate_size_directive: &str,
        rotate_keep_directive: &str,
    ) -> Option<Self> {
        let rotate_size = log_config
            .get_value(rotate_size_directive)
            .and_then(|v| v.as_number())
            .filter(|&v| v > 0)
            .map(|v| v as u64);

        let rotate_keep = log_config
            .get_value(rotate_keep_directive)
            .and_then(|v| v.as_number())
            .filter(|&v| v >= 0)
            .map(|v| v as usize);

        // Only return Some if at least one rotation setting is configured
        if rotate_size.is_some() || rotate_keep.is_some() {
            Some(Self {
                rotate_size,
                rotate_keep,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::FileWriter;

    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlockBuilder, ServerConfigurationDirectiveEntry,
        ServerConfigurationValue,
    };

    fn make_log_config(
        access_log: Option<&str>,
        error_log: Option<&str>,
        rotate_size: Option<u64>,
        rotate_keep: Option<usize>,
    ) -> Arc<ServerConfigurationBlock> {
        let mut builder = ServerConfigurationBlockBuilder::new();

        if let Some(path) = access_log {
            builder = builder.directive_str("access_log", vec![path]);
        }
        if let Some(path) = error_log {
            builder = builder.directive_str("error_log", vec![path]);
        }
        if let Some(size) = rotate_size {
            let entry = ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(size as i64, None)],
                children: None,
                span: None,
            };
            builder = builder.directive("access_log_rotate_size", entry);
        }
        if let Some(keep) = rotate_keep {
            let entry = ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(keep as i64, None)],
                children: None,
                span: None,
            };
            builder = builder.directive("access_log_rotate_keep", entry);
        }

        Arc::new(builder.build())
    }

    #[tokio::test]
    async fn test_rotate_log_file_multiple_rotations() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");

        tokio::fs::write(&log_path, b"first").await.unwrap();
        let path_str = log_path.to_string_lossy().to_string();
        rotate_log_file(&path_str, Some(5)).await.unwrap();

        tokio::fs::write(&log_path, b"second").await.unwrap();
        rotate_log_file(&path_str, Some(5)).await.unwrap();

        let r1 = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        let r2 = tokio::fs::read(format!("{}.2", path_str)).await.unwrap();
        assert_eq!(r1, b"second");
        assert_eq!(r2, b"first");
    }

    #[tokio::test]
    async fn test_rotate_log_file_keep_zero() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        tokio::fs::write(&log_path, b"should be deleted")
            .await
            .unwrap();

        let path_str = log_path.to_string_lossy().to_string();
        rotate_log_file(&path_str, Some(0)).await.unwrap();

        assert!(!tokio::fs::try_exists(&log_path).await.unwrap());
        assert!(!tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_rotate_log_file_no_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        for content in [b"a", b"b", b"c", b"d", b"e"] {
            tokio::fs::write(&log_path, content).await.unwrap();
            rotate_log_file(&path_str, None).await.unwrap();
        }

        for i in 1..=5 {
            assert!(
                tokio::fs::try_exists(format!("{}.{}", path_str, i))
                    .await
                    .unwrap(),
                "Expected {}.{} to exist",
                path_str,
                i
            );
        }
    }

    #[tokio::test]
    async fn test_rotate_log_file_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("nonexistent.log");
        let path_str = log_path.to_string_lossy().to_string();

        let result = rotate_log_file(&path_str, Some(3)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_rotation_config_both_set() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(100), Some(5));
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        )
        .unwrap();
        assert_eq!(rotation.rotate_size, Some(100));
        assert_eq!(rotation.rotate_keep, Some(5));
    }

    #[tokio::test]
    async fn test_read_rotation_config_only_size() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(200), None);
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        )
        .unwrap();
        assert_eq!(rotation.rotate_size, Some(200));
        assert_eq!(rotation.rotate_keep, None);
    }

    #[tokio::test]
    async fn test_read_rotation_config_only_keep() {
        let config = make_log_config(Some("/tmp/access.log"), None, None, Some(3));
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        )
        .unwrap();
        assert_eq!(rotation.rotate_size, None);
        assert_eq!(rotation.rotate_keep, Some(3));
    }

    #[tokio::test]
    async fn test_read_rotation_config_neither_set() {
        let config = make_log_config(Some("/tmp/access.log"), None, None, None);
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        );
        assert!(rotation.is_none());
    }

    #[tokio::test]
    async fn test_read_rotation_config_zero_size() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(0), Some(3));
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        )
        .unwrap();
        assert_eq!(rotation.rotate_size, None);
        assert_eq!(rotation.rotate_keep, Some(3));
    }

    #[tokio::test]
    async fn test_read_rotation_config_zero_keep() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(100), Some(0));
        let rotation = RotationConfig::read_from_config(
            &config,
            "access_log_rotate_size",
            "access_log_rotate_keep",
        )
        .unwrap();
        assert_eq!(rotation.rotate_size, Some(100));
        assert_eq!(rotation.rotate_keep, Some(0));
    }

    #[tokio::test]
    async fn test_file_writer_multiple_rotations() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(3),
            rotate_keep: Some(5),
        });

        let mut writer = FileWriter::new(100);

        for chunk in [b"aaa", b"bbb", b"ccc"] {
            writer
                .write_to_file(&path_str, chunk, rotation)
                .await
                .unwrap();
        }
        writer.flush_all().await.unwrap();

        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.2", path_str))
            .await
            .unwrap());

        let r1 = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        let r2 = tokio::fs::read(format!("{}.2", path_str)).await.unwrap();
        assert_eq!(r1, b"bbb");
        assert_eq!(r2, b"aaa");
    }

    #[tokio::test]
    async fn test_file_writer_current_size_reset_after_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(4),
            rotate_keep: Some(3),
        });

        let mut writer = FileWriter::new(100);

        writer
            .write_to_file(&path_str, b"1234", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 4);

        writer
            .write_to_file(&path_str, b"next", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 4);
    }

    #[tokio::test]
    async fn test_file_writer_existing_file_size_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        tokio::fs::write(&log_path, b"pre-existing content")
            .await
            .unwrap();

        let rotation = Some(RotationConfig {
            rotate_size: Some(25),
            rotate_keep: Some(3),
        });

        let mut writer = FileWriter::new(100);
        writer
            .write_to_file(&path_str, b"more", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 24);
    }

    #[tokio::test]
    async fn test_file_writer_rotation_respects_keep_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(3),
            rotate_keep: Some(2),
        });

        let mut writer = FileWriter::new(100);

        for chunk in [b"aaa", b"bbb", b"ccc", b"ddd"] {
            writer
                .write_to_file(&path_str, chunk, rotation)
                .await
                .unwrap();
        }
        writer.flush_all().await.unwrap();

        assert!(!tokio::fs::try_exists(format!("{}.3", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.2", path_str))
            .await
            .unwrap());
    }
}
