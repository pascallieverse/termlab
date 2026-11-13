//! Local editing of a project's .env, without sending its contents to the daemon.
use std::io::Write;
use std::path::{Path, PathBuf};

fn env_path(cwd: &Path) -> Result<PathBuf, String> {
    if !cwd.is_dir() {
        return Err("The project folder no longer exists.".into());
    }
    let path = cwd.join(".env");
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(
                "The .env path must be a regular file (not a symlink or directory).".into(),
            );
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(path)
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read .env: {e}")),
    }
}

#[tauri::command]
pub fn read_project_env(cwd: PathBuf) -> Result<Option<String>, String> {
    read(&env_path(&cwd)?)
}

#[tauri::command]
pub fn save_project_env(
    cwd: PathBuf,
    content: String,
    original: Option<String>,
) -> Result<(), String> {
    let path = env_path(&cwd)?;
    if read(&path)? != original {
        return Err(
            "The .env file changed on disk. Copy your edits and reopen it before saving.".into(),
        );
    }

    // Write alongside the destination, then atomically replace it. New files are private;
    // existing files keep their permissions.
    let mut temp = tempfile::NamedTempFile::new_in(&cwd).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    if original.is_some() {
        let permissions = std::fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions();
        temp.as_file()
            .set_permissions(permissions)
            .map_err(|e| e.to_string())?;
    }
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    // Check again after preparing the replacement.
    env_path(&cwd)?;
    if read(&path)? != original {
        return Err(
            "The .env file changed on disk. Copy your edits and reopen it before saving.".into(),
        );
    }
    if original.is_none() {
        temp.persist_noclobber(&path).map_err(|e| e.to_string())?;
    } else {
        temp.persist(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_round_trip_and_external_change() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        assert_eq!(read_project_env(cwd.clone()).unwrap(), None);
        let text = "# Comment\r\nTOKEN='a=b'\r\n\r\n".to_string();
        save_project_env(cwd.clone(), text.clone(), None).unwrap();
        assert_eq!(read_project_env(cwd.clone()).unwrap(), Some(text.clone()));
        std::fs::write(cwd.join(".env"), "EXTERNAL=1\n").unwrap();
        assert!(save_project_env(cwd.clone(), "overwrite".into(), Some(text)).is_err());
        assert_eq!(
            read_project_env(cwd.clone()).unwrap().unwrap(),
            "EXTERNAL=1\n"
        );
        save_project_env(cwd.clone(), "".into(), Some("EXTERNAL=1\n".into())).unwrap();
        assert_eq!(read_project_env(cwd).unwrap(), Some("".into()));
    }

    #[test]
    fn rejects_missing_folder_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_project_env(dir.path().join("missing")).is_err());
        std::fs::create_dir(dir.path().join(".env")).unwrap();
        assert!(read_project_env(dir.path().to_path_buf()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_creation_preserves_permissions_and_rejects_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        let path = cwd.join(".env");
        save_project_env(cwd.clone(), "A=1".into(), None).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        save_project_env(cwd.clone(), "A=2".into(), Some("A=1".into())).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        std::fs::remove_file(&path).unwrap();
        symlink(cwd.join("other"), &path).unwrap();
        assert!(save_project_env(cwd, "A=3".into(), None).is_err());
    }
}
