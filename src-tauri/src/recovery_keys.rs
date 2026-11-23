//! Windows account-bound recovery key custody. This module does not authorize a Host run.
use super::*;
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::FromRawHandle,
};
use std::ptr;

#[path = "recovery_keys/portable.rs"]
mod portable;

pub(super) const PORTABLE_BACKUP_BYTES: usize = portable::ENVELOPE_BYTES;

const FILE_NAME: &str = "recovery-key.dpapi.json";
const FORMAT: &str = "ih-recovery-dpapi-v1";
const MAX_RECORD: u64 = 8 * 1024;
const ECC_PRIVATE_BYTES: usize = 104;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const REPARSE_POINT: u32 = 0x400;

type Handle = *mut c_void;

#[repr(C)]
struct Blob {
    length: u32,
    data: *mut u8,
}

impl Blob {
    fn input(bytes: &[u8]) -> Self {
        Self {
            length: bytes.len() as u32,
            data: bytes.as_ptr().cast_mut(),
        }
    }
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: Handle,
    inherit: i32,
}

#[link(name = "crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        input: *const Blob,
        description: *const u16,
        entropy: *const Blob,
        reserved: Handle,
        prompt: Handle,
        flags: u32,
        output: *mut Blob,
    ) -> i32;
    fn CryptUnprotectData(
        input: *const Blob,
        description: *mut *mut u16,
        entropy: *const Blob,
        reserved: Handle,
        prompt: Handle,
        flags: u32,
        output: *mut Blob,
    ) -> i32;
}

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptOpenAlgorithmProvider(
        output: *mut Handle,
        algorithm: *const u16,
        provider: *const u16,
        flags: u32,
    ) -> i32;
    fn BCryptCloseAlgorithmProvider(algorithm: Handle, flags: u32) -> i32;
    fn BCryptGenerateKeyPair(algorithm: Handle, key: *mut Handle, length: u32, flags: u32) -> i32;
    fn BCryptFinalizeKeyPair(key: Handle, flags: u32) -> i32;
    fn BCryptExportKey(
        key: Handle,
        wrapping_key: Handle,
        format: *const u16,
        output: *mut u8,
        output_size: u32,
        written: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptDestroyKey(key: Handle) -> i32;
    fn BCryptImportKeyPair(
        algorithm: Handle,
        import_key: Handle,
        format: *const u16,
        key: *mut Handle,
        input: *const u8,
        input_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptSignHash(
        key: Handle,
        padding: Handle,
        digest: *const u8,
        digest_len: u32,
        signature: *mut u8,
        signature_len: u32,
        written: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptVerifySignature(
        key: Handle,
        padding: Handle,
        digest: *const u8,
        digest_len: u32,
        signature: *const u8,
        signature_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptGenRandom(algorithm: Handle, output: *mut u8, output_len: u32, flags: u32) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        descriptor: *const u16,
        revision: u32,
        output: *mut Handle,
        size: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut SecurityAttributes,
        disposition: u32,
        attributes: u32,
        template: Handle,
    ) -> Handle;
    fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    fn LocalFree(allocation: Handle) -> Handle;
}

fn zero(bytes: &mut [u8]) {
    for byte in bytes {
        // Keep the overwrite observable even if the compiler sees the buffer is about to be freed.
        unsafe { ptr::write_volatile(byte, 0) };
    }
}

struct DpapiData(Blob);

impl DpapiData {
    fn bytes(&self) -> &[u8] {
        // Successful DPAPI calls return a LocalAlloc buffer readable for cbData bytes.
        unsafe { std::slice::from_raw_parts(self.0.data, self.0.length as usize) }
    }
}

impl Drop for DpapiData {
    fn drop(&mut self) {
        if !self.0.data.is_null() {
            unsafe {
                zero(std::slice::from_raw_parts_mut(
                    self.0.data,
                    self.0.length as usize,
                ));
                LocalFree(self.0.data.cast());
            }
        }
    }
}

struct PrivateKey([u8; ECC_PRIVATE_BYTES]);

impl Drop for PrivateKey {
    fn drop(&mut self) {
        zero(&mut self.0);
    }
}

struct Algorithm(Handle);
impl Drop for Algorithm {
    fn drop(&mut self) {
        unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}

struct Key(Handle);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { BCryptDestroyKey(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn cng(result: i32) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(format!("RecoveryKeyProviderFailed:{result:#x}"))
    }
}

fn generate() -> Result<PrivateKey> {
    let mut provider = ptr::null_mut();
    cng(unsafe {
        BCryptOpenAlgorithmProvider(&mut provider, wide("ECDSA_P256").as_ptr(), ptr::null(), 0)
    })?;
    let provider = Algorithm(provider);
    let mut key = ptr::null_mut();
    cng(unsafe { BCryptGenerateKeyPair(provider.0, &mut key, 256, 0) })?;
    let key = Key(key);
    cng(unsafe { BCryptFinalizeKeyPair(key.0, 0) })?;
    let mut private = PrivateKey([0; ECC_PRIVATE_BYTES]);
    let mut written = 0;
    cng(unsafe {
        BCryptExportKey(
            key.0,
            ptr::null_mut(),
            wide("ECCPRIVATEBLOB").as_ptr(),
            private.0.as_mut_ptr(),
            ECC_PRIVATE_BYTES as u32,
            &mut written,
            0,
        )
    })?;
    if written != ECC_PRIVATE_BYTES as u32 {
        return Err("RecoveryKeyInvalidFormat".into());
    }
    Ok(private)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if !is_hex(value, N * 2) {
        return None;
    }
    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn import_key(algorithm: &Algorithm, format: &str, bytes: &[u8]) -> Result<Key> {
    let mut imported = ptr::null_mut();
    cng(unsafe {
        BCryptImportKeyPair(
            algorithm.0,
            ptr::null_mut(),
            wide(format).as_ptr(),
            &mut imported,
            bytes.as_ptr(),
            bytes.len() as u32,
            0,
        )
    })?;
    Ok(Key(imported))
}

fn payload_digest(payload: &[u8]) -> Result<[u8; 32]> {
    decode_hex(&content_revision::revision(payload)?)
        .ok_or_else(|| "RecoveryDigestInvalidFormat".into())
}

/// Sign a Host challenge only with the exact key generation and public key pinned by that Host.
pub(super) fn sign(
    root: &Path,
    workspace: &str,
    expected: &PublicKey,
    payload: &[u8],
) -> Result<String> {
    let _writer = writer_guard(root)?;
    let record = load(root, workspace)?.ok_or("RecoveryKeyNotFound")?;
    if record.public() != *expected {
        return Err("RecoveryKeyGenerationChanged".into());
    }
    let private = dpapi(&record.protected, &record.entropy(), false)?;
    let mut provider = ptr::null_mut();
    cng(unsafe {
        BCryptOpenAlgorithmProvider(&mut provider, wide("ECDSA_P256").as_ptr(), ptr::null(), 0)
    })?;
    let provider = Algorithm(provider);
    let signer = import_key(&provider, "ECCPRIVATEBLOB", private.bytes())?;
    let digest = payload_digest(payload)?;
    let mut signature = [0u8; 64];
    let mut written = 0;
    cng(unsafe {
        BCryptSignHash(
            signer.0,
            ptr::null_mut(),
            digest.as_ptr(),
            digest.len() as u32,
            signature.as_mut_ptr(),
            signature.len() as u32,
            &mut written,
            0,
        )
    })?;
    if written != signature.len() as u32 {
        return Err("RecoverySignatureInvalidLength".into());
    }
    Ok(hex(&signature))
}

/// Verify against the Host's pinned public key; the mutable workspace key is never consulted.
pub(super) fn verify(public: &PublicKey, payload: &[u8], signature: &str) -> Result<bool> {
    let coordinates: [u8; 64] = decode_hex(&public.public_key).ok_or("RecoveryKeyInvalidFormat")?;
    if public.generation == 0 {
        return Err("RecoveryKeyInvalidFormat".into());
    }
    let signature: [u8; 64] = decode_hex(signature).ok_or("RecoverySignatureInvalidFormat")?;
    let mut provider = ptr::null_mut();
    cng(unsafe {
        BCryptOpenAlgorithmProvider(&mut provider, wide("ECDSA_P256").as_ptr(), ptr::null(), 0)
    })?;
    let provider = Algorithm(provider);
    let mut public_blob = [0u8; 72];
    public_blob[..8].copy_from_slice(b"ECS1\x20\x00\x00\x00");
    public_blob[8..].copy_from_slice(&coordinates);
    let verifier = import_key(&provider, "ECCPUBLICBLOB", &public_blob)?;
    let digest = payload_digest(payload)?;
    const STATUS_INVALID_SIGNATURE: i32 = 0xc000_a000u32 as i32;
    match unsafe {
        BCryptVerifySignature(
            verifier.0,
            ptr::null_mut(),
            digest.as_ptr(),
            digest.len() as u32,
            signature.as_ptr(),
            signature.len() as u32,
            0,
        )
    } {
        0 => Ok(true),
        STATUS_INVALID_SIGNATURE => Ok(false),
        error => Err(format!("RecoveryKeyProviderFailed:{error:#x}")),
    }
}

/// 256 random bits for a single-use, short-lived recovery challenge.
pub(super) fn nonce() -> Result<String> {
    let mut bytes = [0u8; 32];
    cng(unsafe { BCryptGenRandom(ptr::null_mut(), bytes.as_mut_ptr(), bytes.len() as u32, 2) })?;
    Ok(hex(&bytes))
}

fn public_coordinate(private: &[u8]) -> Result<String> {
    if private.len() != ECC_PRIVATE_BYTES || &private[..8] != b"ECS2\x20\x00\x00\x00" {
        return Err("RecoveryKeyInvalidFormat".into());
    }
    // Win32 stores P-256 x, y and d in big-endian order following the header.
    Ok(hex(&private[8..72]))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicKey {
    pub(super) generation: u64,
    pub(super) public_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    format: String,
    workspace: String,
    generation: u64,
    public_key: String,
    protected: Vec<u8>,
}

impl Record {
    fn entropy(&self) -> Vec<u8> {
        format!(
            "{}:{}:{}:{}",
            FORMAT, self.workspace, self.generation, self.public_key
        )
        .into_bytes()
    }

    fn public(&self) -> PublicKey {
        PublicKey {
            generation: self.generation,
            public_key: self.public_key.clone(),
        }
    }

    fn verify(&self, workspace: &str) -> Result<()> {
        if self.format != FORMAT
            || self.workspace != workspace
            || self.generation == 0
            || !is_hex(&self.public_key, 128)
            || self.protected.is_empty()
            || self.protected.len() > 4096
        {
            return Err("RecoveryKeyInvalidRecord".into());
        }
        let plaintext = dpapi(&self.protected, &self.entropy(), false)?;
        if public_coordinate(plaintext.bytes())? != self.public_key {
            return Err("RecoveryKeyPublicMismatch".into());
        }
        Ok(())
    }
}

fn dpapi(bytes: &[u8], entropy: &[u8], protect: bool) -> Result<DpapiData> {
    const UI_FORBIDDEN: u32 = 1;
    let mut output = DpapiData(Blob {
        length: 0,
        data: ptr::null_mut(),
    });
    let input = Blob::input(bytes);
    let entropy = Blob::input(entropy);
    let success = if protect {
        unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                UI_FORBIDDEN,
                &mut output.0,
            )
        }
    } else {
        unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    if success == 0 || output.0.data.is_null() || output.0.length == 0 {
        return Err(if protect {
            "RecoveryKeyProtectionFailed"
        } else {
            "RecoveryKeyUnlockFailed"
        }
        .into());
    }
    Ok(output)
}

fn load(root: &Path, workspace: &str) -> Result<Option<Record>> {
    if !is_hex(workspace, 32) {
        return Err("InvalidRecoveryWorkspaceId".into());
    }
    no_redirect(root)?;
    let path = root.join(FILE_NAME);
    let mut file = match OpenOptions::new()
        .read(true)
        .share_mode(1 | 4)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("open-recovery-key", &path, error)),
    };
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || metadata.len() > MAX_RECORD
    {
        return Err("RecoveryKeyUnsafeRecord".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err("RecoveryKeyUnsafeRecord".into());
    }
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| "RecoveryKeyInvalidRecord")?;
    record.verify(workspace)?;
    Ok(Some(record))
}

fn protect_directory(root: &Path) -> Result<File> {
    no_redirect(root)?;
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
        .open(root)
        .map_err(|error| file_error("pin-recovery-directory", root, error))?;
    let metadata = handle.metadata().map_err(fail)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("RecoveryKeyUnsafeDirectory".into());
    }
    Ok(handle)
}

fn create_private_file(path: &Path) -> Result<File> {
    // Explicit owner and protected DACL for the current Windows account, even
    // when an elevated token's default owner would be Administrators.
    let sid = host_pipe::current_sid().map_err(fail)?;
    let sddl = wide(&format!("O:{sid}D:P(A;;FA;;;{sid})"));
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(file_error(
            "protect-recovery-key-file",
            path,
            io::Error::last_os_error(),
        ));
    }
    let mut security = SecurityAttributes {
        length: size_of::<SecurityAttributes>() as u32,
        descriptor,
        inherit: 0,
    };
    let wide_path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            0x4000_0000,
            0,
            &mut security,
            1,
            0x80,
            ptr::null_mut(),
        )
    };
    let error = io::Error::last_os_error();
    unsafe { LocalFree(descriptor) };
    if handle as isize == -1 {
        return Err(file_error("create-protected-recovery-key", path, error));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn persist(root: &Path, record: &Record, replace: bool) -> Result<()> {
    let path = root.join(FILE_NAME);
    let stage = root.join(format!(
        "recovery-key-{}-{}.tmp",
        std::process::id(),
        clock()
    ));
    let mut output = create_private_file(&stage)?;
    let write_result = serde_json::to_writer(&mut output, record)
        .map_err(fail)
        .and_then(|()| output.sync_all().map_err(fail));
    drop(output);
    let result = write_result.and_then(|()| {
        let from: Vec<_> = stage.as_os_str().encode_wide().chain([0]).collect();
        let to: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8 | u32::from(replace)) } == 0 {
            return Err(file_error(
                "publish-recovery-key",
                &path,
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    });
    if let Err(error) = result {
        return match fs::remove_file(&stage) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!(
                "{error}; {}",
                file_error("discard-recovery-key-stage", &stage, cleanup)
            )),
        };
    }
    Ok(())
}

pub(super) fn current(root: &Path, workspace: &str) -> Result<Option<PublicKey>> {
    Ok(load(root, workspace)?.map(|key| key.public()))
}

pub(super) fn backup(
    root: &Path,
    workspace: &str,
    expected_generation: u64,
    password: &[u8],
) -> Result<Vec<u8>> {
    let _directory = protect_directory(root)?;
    let _writer = writer_guard(root)?;
    let record = load(root, workspace)?.ok_or("RecoveryKeyNotFound")?;
    if record.generation != expected_generation {
        return Err("RecoveryKeyGenerationChanged".into());
    }
    let plaintext = dpapi(&record.protected, &record.entropy(), false)?;
    portable::encrypt(workspace, &record.public(), password, plaintext.bytes())
}

pub(super) fn inspect_backup(workspace: &str, bytes: &[u8], password: &[u8]) -> Result<PublicKey> {
    let (public, _) = portable::decrypt(workspace, bytes, password)?;
    Ok(public)
}

pub(super) fn restore_backup(
    root: &Path,
    workspace: &str,
    expected: &PublicKey,
    bytes: &[u8],
    password: &[u8],
) -> Result<PublicKey> {
    let (public, private) = portable::decrypt(workspace, bytes, password)?;
    if &public != expected {
        return Err("RecoveryBackupIdentityChanged".into());
    }
    let _directory = protect_directory(root)?;
    let _writer = writer_guard(root)?;
    if let Some(current) = load(root, workspace)? {
        if current.public() != public {
            return Err("RecoveryKeyGenerationChanged".into());
        }
        return Ok(public);
    }
    let mut record = Record {
        format: FORMAT.into(),
        workspace: workspace.into(),
        generation: public.generation,
        public_key: public.public_key.clone(),
        protected: Vec::new(),
    };
    record.protected = dpapi(private.bytes(), &record.entropy(), true)?
        .bytes()
        .to_vec();
    record.verify(workspace)?;
    persist(root, &record, false)?;
    Ok(public)
}

fn writer_guard(root: &Path) -> Result<std::os::windows::io::OwnedHandle> {
    let path = root.as_os_str().to_string_lossy().to_lowercase();
    let digest = content_revision::revision(path.as_bytes())?;
    gui_singleton::acquire_named(&format!(r"Global\InstanceHarborRecoveryKey-{digest}"))
        .map_err(fail)?
        .ok_or_else(|| "RecoveryKeyRotationBusy".into())
}

/// The caller supplies the workspace's stable identity. An existing Host must
/// pin its own key epoch at start and retain it across controller replacement.
pub(super) fn rotate(root: &Path, workspace: &str, expected: Option<u64>) -> Result<PublicKey> {
    let _directory = protect_directory(root)?;
    let _writer = writer_guard(root)?;
    let current = load(root, workspace)?;
    if current.as_ref().map(|key| key.generation) != expected {
        return Err("RecoveryKeyGenerationChanged".into());
    }
    let generation = expected
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("RecoveryKeyGenerationExhausted")?;
    let private = generate()?;
    let mut record = Record {
        format: FORMAT.into(),
        workspace: workspace.into(),
        generation,
        public_key: public_coordinate(&private.0)?,
        protected: Vec::new(),
    };
    record.protected = dpapi(&private.0, &record.entropy(), true)?.bytes().to_vec();
    record.verify(workspace)?;
    persist(root, &record, expected.is_some())?;
    Ok(record.public())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const WORKSPACE: &str = "18bbd29f20da4156a9f962c6056f9784";
    const OTHER: &str = "f729ab7824f1486f8587158955e75777";
    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let parent = std::env::temp_dir().canonicalize().unwrap();
            let root = parent.join(format!(
                "ih-recovery-test-{}-{}-{}",
                std::process::id(),
                clock(),
                FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn creates_reads_and_rotates_the_windows_protected_key() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        assert_eq!(current(root, WORKSPACE).unwrap(), None);
        let first = rotate(root, WORKSPACE, None).unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(current(root, WORKSPACE).unwrap(), Some(first.clone()));
        assert_eq!(
            rotate(root, WORKSPACE, None).unwrap_err(),
            "RecoveryKeyGenerationChanged"
        );
        let second = rotate(root, WORKSPACE, Some(1)).unwrap();
        assert_eq!(second.generation, 2);
        assert_ne!(first.public_key, second.public_key);
        assert_eq!(current(root, WORKSPACE).unwrap(), Some(second));
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }

    #[test]
    fn portable_restore_republishes_dpapi_and_refuses_overwriting_other_key() {
        let origin = Fixture::new();
        let destination = Fixture::new();
        let password = b"public-test-passphrase";
        let key = rotate(&origin.0, WORKSPACE, None).unwrap();
        assert_eq!(
            backup(&origin.0, WORKSPACE, 2, password).unwrap_err(),
            "RecoveryKeyGenerationChanged"
        );
        let bytes = backup(&origin.0, WORKSPACE, 1, password).unwrap();
        assert_eq!(bytes.len(), PORTABLE_BACKUP_BYTES);
        assert_eq!(inspect_backup(WORKSPACE, &bytes, password).unwrap(), key);
        let wrong_key = PublicKey {
            generation: 2,
            public_key: key.public_key.clone(),
        };
        assert_eq!(
            restore_backup(&destination.0, WORKSPACE, &wrong_key, &bytes, password).unwrap_err(),
            "RecoveryBackupIdentityChanged"
        );
        assert_eq!(current(&destination.0, WORKSPACE).unwrap(), None);
        assert_eq!(
            restore_backup(&destination.0, WORKSPACE, &key, &bytes, password).unwrap(),
            key
        );
        let published = fs::read(destination.0.join(FILE_NAME)).unwrap();
        assert_eq!(
            current(&destination.0, WORKSPACE).unwrap(),
            Some(key.clone())
        );
        assert_eq!(
            restore_backup(&destination.0, WORKSPACE, &key, &bytes, password).unwrap(),
            key
        );
        assert_eq!(fs::read(destination.0.join(FILE_NAME)).unwrap(), published);
        let rotated = rotate(&destination.0, WORKSPACE, Some(1)).unwrap();
        assert_eq!(
            restore_backup(&destination.0, WORKSPACE, &key, &bytes, password).unwrap_err(),
            "RecoveryKeyGenerationChanged"
        );
        assert_eq!(current(&destination.0, WORKSPACE).unwrap(), Some(rotated));
    }

    #[test]
    fn authenticates_workspace_generation_and_protected_bytes() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        rotate(root, WORKSPACE, None).unwrap();
        assert_eq!(
            current(root, OTHER).unwrap_err(),
            "RecoveryKeyInvalidRecord"
        );
        let path = root.join(FILE_NAME);
        let original = fs::read(&path).unwrap();
        let mut tampered: Record = serde_json::from_slice(&original).unwrap();
        tampered.generation = 2;
        fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert_eq!(
            current(root, WORKSPACE).unwrap_err(),
            "RecoveryKeyUnlockFailed"
        );
        let mut tampered: Record = serde_json::from_slice(&original).unwrap();
        let last = tampered.protected.last_mut().unwrap();
        *last ^= 1;
        fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(current(root, WORKSPACE).is_err());
        assert!(rotate(root, WORKSPACE, None).is_err());
    }

    #[test]
    fn refuses_a_symbolic_link_at_the_key_store() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let unrelated = root.join("unrelated");
        fs::write(&unrelated, b"untouched").unwrap();
        match std::os::windows::fs::symlink_file(&unrelated, root.join(FILE_NAME)) {
            Ok(()) => {
                assert_eq!(
                    current(root, WORKSPACE).unwrap_err(),
                    "RecoveryKeyUnsafeRecord"
                );
                assert_eq!(
                    rotate(root, WORKSPACE, None).unwrap_err(),
                    "RecoveryKeyUnsafeRecord"
                );
            }
            // Developer Mode and elevation are not prerequisites for normal key custody.
            Err(error) if error.raw_os_error() == Some(1314) => return,
            Err(error) => panic!("Cannot build recovery symlink fixture: {error}"),
        }
        assert_eq!(fs::read(unrelated).unwrap(), b"untouched");
    }

    #[test]
    fn competing_rotations_cannot_both_use_the_same_generation() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        rotate(root, WORKSPACE, None).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let outcomes = thread::scope(|scope| {
            let rotate_from_first = || {
                barrier.wait();
                rotate(root, WORKSPACE, Some(1))
            };
            let contender = scope.spawn(rotate_from_first);
            [rotate_from_first(), contender.join().unwrap()]
        });
        assert_eq!(outcomes.iter().filter(|value| value.is_ok()).count(), 1);
        let error = outcomes
            .iter()
            .find_map(|value| value.as_ref().err())
            .unwrap();
        assert!(matches!(
            error.as_str(),
            "RecoveryKeyRotationBusy" | "RecoveryKeyGenerationChanged"
        ));
        assert_eq!(current(root, WORKSPACE).unwrap().unwrap().generation, 2);
    }

    #[test]
    fn signs_only_for_the_pinned_key_and_detects_tampering() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let first = rotate(root, WORKSPACE, None).unwrap();
        let payload = b"IH-Recover-v1 isolated run and actual peer";
        let signature = sign(root, WORKSPACE, &first, payload).unwrap();
        assert_eq!(signature.len(), 128);
        assert!(verify(&first, payload, &signature).unwrap());
        assert!(!verify(&first, b"altered challenge", &signature).unwrap());
        let mut altered_signature = signature.clone();
        altered_signature.replace_range(0..2, if &signature[..2] == "00" { "01" } else { "00" });
        assert!(!verify(&first, payload, &altered_signature).unwrap());
        assert_eq!(
            verify(&first, payload, "invalid").unwrap_err(),
            "RecoverySignatureInvalidFormat"
        );

        let second = rotate(root, WORKSPACE, Some(first.generation)).unwrap();
        assert_eq!(
            sign(root, WORKSPACE, &first, payload).unwrap_err(),
            "RecoveryKeyGenerationChanged"
        );
        assert!(!verify(&second, payload, &signature).unwrap());
        let second_signature = sign(root, WORKSPACE, &second, payload).unwrap();
        assert!(verify(&second, payload, &second_signature).unwrap());
    }

    #[test]
    fn random_recovery_nonces_are_full_length_and_distinct() {
        let first = nonce().unwrap();
        let second = nonce().unwrap();
        assert_eq!(first.len(), 64);
        assert!(is_hex(&first, 64));
        assert_ne!(first, second);
    }
}
