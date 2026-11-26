//! Opaque optimistic-concurrency tokens without exposing file contents to the renderer.
use super::*;

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptOpenAlgorithmProvider(
        handle: *mut *mut c_void,
        algorithm: *const u16,
        implementation: *const u16,
        flags: u32,
    ) -> i32;
    fn BCryptHash(
        handle: *mut c_void,
        secret: *const u8,
        secret_length: u32,
        input: *const u8,
        length: u32,
        output: *mut u8,
        output_length: u32,
    ) -> i32;
    fn BCryptCreateHash(
        algorithm: *mut c_void,
        hash: *mut *mut c_void,
        object: *mut u8,
        object_length: u32,
        secret: *const u8,
        secret_length: u32,
        flags: u32,
    ) -> i32;
    fn BCryptHashData(hash: *mut c_void, input: *const u8, length: u32, flags: u32) -> i32;
    fn BCryptFinishHash(hash: *mut c_void, output: *mut u8, length: u32, flags: u32) -> i32;
    fn BCryptDestroyHash(hash: *mut c_void) -> i32;
    fn BCryptCloseAlgorithmProvider(handle: *mut c_void, flags: u32) -> i32;
}

pub(super) fn revision(bytes: &[u8]) -> Result<String> {
    let length = u32::try_from(bytes.len()).map_err(|_| "SettingsFileTooLarge")?;
    let mut handle = std::ptr::null_mut();
    let algorithm = [83u16, 72, 65, 50, 53, 54, 0];
    let opened = unsafe {
        BCryptOpenAlgorithmProvider(&mut handle, algorithm.as_ptr(), std::ptr::null(), 0)
    };
    if opened != 0 {
        return Err(format!("SettingsRevisionUnavailable:{opened:#x}"));
    }
    let mut output = [0u8; 32];
    let hashed = unsafe {
        BCryptHash(
            handle,
            std::ptr::null(),
            0,
            bytes.as_ptr(),
            length,
            output.as_mut_ptr(),
            output.len() as u32,
        )
    };
    let closed = unsafe { BCryptCloseAlgorithmProvider(handle, 0) };
    if hashed != 0 || closed != 0 {
        return Err(format!(
            "SettingsRevisionUnavailable:{hashed:#x}:{closed:#x}"
        ));
    }
    Ok(output.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn file_revision(path: &Path) -> Result<String> {
    file_digest(path, "SHA256", 32)
}

/// The MD5 of a file. Only for comparing a download with an official source
/// that publishes no stronger hash (Purpur, #41); identity stays SHA-256.
pub(super) fn file_md5(path: &Path) -> Result<String> {
    file_digest(path, "MD5", 16)
}

fn file_digest(path: &Path, algorithm_name: &str, size: usize) -> Result<String> {
    no_redirect(path)?;
    let mut file = File::open(path).map_err(|error| file_error("open-core", path, error))?;
    digest_open(&mut file, path, algorithm_name, size)
}

/// The SHA-256 of a file the caller already holds open, so the bytes checked are the
/// bytes the caller then uses (#65). Reads from the current position.
pub(super) fn open_file_revision(file: &mut File, path: &Path) -> Result<String> {
    digest_open(file, path, "SHA256", 32)
}

fn digest_open(file: &mut File, path: &Path, algorithm_name: &str, size: usize) -> Result<String> {
    if file.metadata().map_err(fail)?.len() > 512 * 1024 * 1024 {
        return Err("CoreArtifactTooLarge".into());
    }
    struct Algorithm(*mut c_void);
    impl Drop for Algorithm {
        fn drop(&mut self) {
            unsafe {
                BCryptCloseAlgorithmProvider(self.0, 0);
            }
        }
    }
    struct Hash(*mut c_void);
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                BCryptDestroyHash(self.0);
            }
        }
    }
    let check = |status: i32| {
        if status == 0 {
            Ok(())
        } else {
            Err(format!("CoreHashUnavailable:{status:#x}"))
        }
    };
    let mut handle = std::ptr::null_mut();
    let name: Vec<u16> = algorithm_name.encode_utf16().chain([0]).collect();
    check(unsafe { BCryptOpenAlgorithmProvider(&mut handle, name.as_ptr(), std::ptr::null(), 0) })?;
    let algorithm = Algorithm(handle);
    let mut hash = std::ptr::null_mut();
    check(unsafe {
        BCryptCreateHash(
            algorithm.0,
            &mut hash,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            0,
            0,
        )
    })?;
    let hash = Hash(hash);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0usize;
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|error| file_error("read-core", path, error))?;
        if length == 0 {
            break;
        }
        total += length;
        if total > 512 * 1024 * 1024 {
            return Err("CoreArtifactTooLarge".into());
        }
        check(unsafe { BCryptHashData(hash.0, buffer.as_ptr(), length as u32, 0) })?;
    }
    let mut output = vec![0u8; size];
    check(unsafe { BCryptFinishHash(hash.0, output.as_mut_ptr(), output.len() as u32, 0) })?;
    Ok(output
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_and_bytes_match_over_multiple_buffers() {
        let temp = std::env::temp_dir().join(format!(
            "ih-content-revision-{}-{}",
            std::process::id(),
            clock()
        ));
        let data = vec![42u8; 128 * 1024 + 1];
        fs::write(&temp, &data).unwrap();
        let path = temp.canonicalize().unwrap();
        let hashed = file_revision(&path);
        fs::remove_file(path).unwrap();
        assert_eq!(hashed.unwrap(), revision(&data).unwrap());
    }

    #[test]
    fn standard_md5_vector_of_a_file() {
        let temp = std::env::temp_dir().join(format!("ih-content-md5-{}-{}", std::process::id(), clock()));
        fs::write(&temp, b"abc").unwrap();
        let path = temp.canonicalize().unwrap();
        let hashed = file_md5(&path);
        fs::remove_file(path).unwrap();
        assert_eq!(hashed.unwrap(), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn standard_sha256_vector() {
        assert_eq!(
            revision(b"abc").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
