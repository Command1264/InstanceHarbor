//! Fixed-size v1 encrypted envelope. The entire header is AES-GCM associated data.
use super::*;

const MAGIC: &[u8; 8] = b"IHRKEY\x01\0";
const ITERATIONS: u64 = 600_000;
const BCRYPT_ALG_HANDLE_HMAC_FLAG: u32 = 8;
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
const STATUS_AUTH_TAG_MISMATCH: i32 = 0xc000_a002u32 as i32;
const WORKSPACE_START: usize = MAGIC.len();
const GENERATION_START: usize = WORKSPACE_START + 32;
const PUBLIC_START: usize = GENERATION_START + 8;
const SALT_START: usize = PUBLIC_START + 128;
const NONCE_START: usize = SALT_START + 16;
const HEADER_BYTES: usize = NONCE_START + 12;
const TAG_START: usize = HEADER_BYTES + ECC_PRIVATE_BYTES;
const TAG_BYTES: usize = 16;
pub(super) const ENVELOPE_BYTES: usize = TAG_START + TAG_BYTES;

#[repr(C)]
struct AuthenticatedCipherMode {
    size: u32,
    version: u32,
    nonce: *mut u8,
    nonce_len: u32,
    auth_data: *mut u8,
    auth_data_len: u32,
    tag: *mut u8,
    tag_len: u32,
    mac_context: *mut u8,
    mac_context_len: u32,
    aad_len: u32,
    data_len: u64,
    flags: u32,
}

type Cipher = unsafe extern "system" fn(
    Handle,
    *const u8,
    u32,
    *mut AuthenticatedCipherMode,
    *mut u8,
    u32,
    *mut u8,
    u32,
    *mut u32,
    u32,
) -> i32;

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: Handle, output: *mut u8, output_len: u32, flags: u32) -> i32;
    fn BCryptDeriveKeyPBKDF2(
        algorithm: Handle,
        password: *const u8,
        password_len: u32,
        salt: *const u8,
        salt_len: u32,
        iterations: u64,
        output: *mut u8,
        output_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptGetProperty(
        object: Handle,
        property: *const u16,
        output: *mut u8,
        output_len: u32,
        written: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptSetProperty(
        object: Handle,
        property: *const u16,
        input: *const u8,
        input_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptGenerateSymmetricKey(
        algorithm: Handle,
        key: *mut Handle,
        key_object: *mut u8,
        key_object_len: u32,
        secret: *const u8,
        secret_len: u32,
        flags: u32,
    ) -> i32;
    fn BCryptEncrypt(
        key: Handle,
        input: *const u8,
        input_len: u32,
        info: *mut AuthenticatedCipherMode,
        iv: *mut u8,
        iv_len: u32,
        output: *mut u8,
        output_len: u32,
        written: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptDecrypt(
        key: Handle,
        input: *const u8,
        input_len: u32,
        info: *mut AuthenticatedCipherMode,
        iv: *mut u8,
        iv_len: u32,
        output: *mut u8,
        output_len: u32,
        written: *mut u32,
        flags: u32,
    ) -> i32;
    fn BCryptImportKeyPair(
        provider: Handle,
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
}

pub(super) struct Secret(Vec<u8>);

impl Secret {
    fn zeroed(length: usize) -> Self {
        Self(vec![0; length])
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zero(&mut self.0);
    }
}

fn open_algorithm(name: &str, flags: u32) -> Result<Algorithm> {
    let mut provider = ptr::null_mut();
    cng(unsafe {
        BCryptOpenAlgorithmProvider(&mut provider, wide(name).as_ptr(), ptr::null(), flags)
    })?;
    Ok(Algorithm(provider))
}

fn validate_password(password: &[u8]) -> Result<()> {
    if password.len() > 1024 {
        return Err("RecoveryBackupInvalidPassword".into());
    }
    let value = std::str::from_utf8(password).map_err(|_| "RecoveryBackupInvalidPassword")?;
    if value.chars().count() < 12 || value.trim().is_empty() {
        return Err("RecoveryBackupInvalidPassword".into());
    }
    Ok(())
}

fn import_ecc_key(provider: &Algorithm, format: &str, bytes: &[u8]) -> Result<Key> {
    let mut imported = ptr::null_mut();
    cng(unsafe {
        BCryptImportKeyPair(
            provider.0,
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

fn verify_pair(private: &Secret) -> Result<()> {
    let provider = open_algorithm("ECDSA_P256", 0)?;
    let signer = import_ecc_key(&provider, "ECCPRIVATEBLOB", &private.0)?;
    let mut public = [0u8; 72];
    public.copy_from_slice(&private.0[..72]);
    public[3] = b'1'; // ECDSA P-256 public BLOB magic is ECS1.
    let verifier = import_ecc_key(&provider, "ECCPUBLICBLOB", &public)?;
    let digest: [u8; 32] = *b"IHR v1 key pair validity check\0\0";
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
        return Err("RecoveryBackupInvalidSignatureLength".into());
    }
    cng(unsafe {
        BCryptVerifySignature(
            verifier.0,
            ptr::null_mut(),
            digest.as_ptr(),
            digest.len() as u32,
            signature.as_ptr(),
            written,
            0,
        )
    })
    .map_err(|_| "RecoveryKeyPublicMismatch".into())
}

fn derive_key(password: &[u8], salt: &[u8]) -> Result<Secret> {
    validate_password(password)?;
    let hash = open_algorithm("SHA256", BCRYPT_ALG_HANDLE_HMAC_FLAG)?;
    let mut secret = Secret::zeroed(32);
    cng(unsafe {
        BCryptDeriveKeyPBKDF2(
            hash.0,
            password.as_ptr(),
            password.len() as u32,
            salt.as_ptr(),
            salt.len() as u32,
            ITERATIONS,
            secret.0.as_mut_ptr(),
            secret.0.len() as u32,
            0,
        )
    })?;
    Ok(secret)
}

struct AesKey {
    // CNG references its caller-owned object until the key handle is destroyed.
    key: Key,
    _object: Secret,
    _algorithm: Algorithm,
}

impl AesKey {
    fn from_password(password: &[u8], salt: &[u8]) -> Result<Self> {
        let secret = derive_key(password, salt)?;
        let algorithm = open_algorithm("AES", 0)?;
        let chain_mode = wide("ChainingModeGCM");
        cng(unsafe {
            BCryptSetProperty(
                algorithm.0,
                wide("ChainingMode").as_ptr(),
                chain_mode.as_ptr().cast(),
                (chain_mode.len() * 2) as u32,
                0,
            )
        })?;
        let mut object_len = 0u32;
        let mut written = 0;
        cng(unsafe {
            BCryptGetProperty(
                algorithm.0,
                wide("ObjectLength").as_ptr(),
                (&mut object_len as *mut u32).cast(),
                size_of::<u32>() as u32,
                &mut written,
                0,
            )
        })?;
        if written != size_of::<u32>() as u32 || !(1..=65536).contains(&object_len) {
            return Err("RecoveryBackupInvalidKeyObject".into());
        }
        let mut object = Secret::zeroed(object_len as usize);
        let mut key = ptr::null_mut();
        cng(unsafe {
            BCryptGenerateSymmetricKey(
                algorithm.0,
                &mut key,
                object.0.as_mut_ptr(),
                object_len,
                secret.0.as_ptr(),
                secret.0.len() as u32,
                0,
            )
        })?;
        Ok(Self {
            key: Key(key),
            _object: object,
            _algorithm: algorithm,
        })
    }

    fn apply(
        &self,
        cipher: Cipher,
        header: &[u8],
        tag: &mut [u8; TAG_BYTES],
        input: &[u8],
        output: &mut [u8],
    ) -> Result<()> {
        let nonce = &header[NONCE_START..HEADER_BYTES];
        let mut mode = AuthenticatedCipherMode {
            size: size_of::<AuthenticatedCipherMode>() as u32,
            version: 1,
            nonce: nonce.as_ptr().cast_mut(),
            nonce_len: nonce.len() as u32,
            auth_data: header.as_ptr().cast_mut(),
            auth_data_len: header.len() as u32,
            tag: tag.as_mut_ptr(),
            tag_len: TAG_BYTES as u32,
            mac_context: ptr::null_mut(),
            mac_context_len: 0,
            aad_len: 0,
            data_len: 0,
            flags: 0,
        };
        let mut written = 0;
        let result = unsafe {
            cipher(
                self.key.0,
                input.as_ptr(),
                input.len() as u32,
                &mut mode,
                ptr::null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut written,
                0,
            )
        };
        if result == STATUS_AUTH_TAG_MISMATCH {
            return Err("RecoveryBackupAuthenticationFailed".into());
        }
        cng(result)?;
        if written as usize != output.len() {
            return Err("RecoveryBackupInvalidLength".into());
        }
        Ok(())
    }
}

pub(super) fn encrypt(
    workspace: &str,
    public: &PublicKey,
    password: &[u8],
    private: &[u8],
) -> Result<Vec<u8>> {
    if !is_hex(workspace, 32)
        || public.generation == 0
        || public_coordinate(private)? != public.public_key
    {
        return Err("RecoveryBackupInvalidHeader".into());
    }
    let mut output = vec![0u8; ENVELOPE_BYTES];
    output[..WORKSPACE_START].copy_from_slice(MAGIC);
    output[WORKSPACE_START..GENERATION_START].copy_from_slice(workspace.as_bytes());
    output[GENERATION_START..PUBLIC_START].copy_from_slice(&public.generation.to_be_bytes());
    output[PUBLIC_START..SALT_START].copy_from_slice(public.public_key.as_bytes());
    let random = &mut output[SALT_START..HEADER_BYTES];
    cng(unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            random.as_mut_ptr(),
            random.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    })?;
    let (header, payload) = output.split_at_mut(HEADER_BYTES);
    let (ciphertext, tag) = payload.split_at_mut(ECC_PRIVATE_BYTES);
    let key = AesKey::from_password(password, &header[SALT_START..NONCE_START])?;
    key.apply(
        BCryptEncrypt,
        header,
        tag.try_into().expect("fixed tag length"),
        private,
        ciphertext,
    )?;
    Ok(output)
}

pub(super) fn decrypt(
    workspace: &str,
    bytes: &[u8],
    password: &[u8],
) -> Result<(PublicKey, Secret)> {
    if !is_hex(workspace, 32) {
        return Err("InvalidRecoveryWorkspaceId".into());
    }
    if bytes.len() != ENVELOPE_BYTES || &bytes[..WORKSPACE_START] != MAGIC {
        return Err("RecoveryBackupInvalidHeader".into());
    }
    let generation = u64::from_be_bytes(
        bytes[GENERATION_START..PUBLIC_START]
            .try_into()
            .expect("fixed header"),
    );
    let public_key = std::str::from_utf8(&bytes[PUBLIC_START..SALT_START])
        .map_err(|_| "RecoveryBackupInvalidHeader")?;
    if generation == 0 || !is_hex(public_key, 128) {
        return Err("RecoveryBackupInvalidHeader".into());
    }
    let key = AesKey::from_password(password, &bytes[SALT_START..NONCE_START])?;
    let mut private = Secret::zeroed(ECC_PRIVATE_BYTES);
    let mut tag = bytes[TAG_START..].try_into().expect("fixed tag length");
    key.apply(
        BCryptDecrypt,
        &bytes[..HEADER_BYTES],
        &mut tag,
        &bytes[HEADER_BYTES..TAG_START],
        &mut private.0,
    )?;
    if &bytes[WORKSPACE_START..GENERATION_START] != workspace.as_bytes() {
        return Err("RecoveryBackupWorkspaceMismatch".into());
    }
    if public_coordinate(&private.0)? != public_key {
        return Err("RecoveryKeyPublicMismatch".into());
    }
    verify_pair(&private)?;
    Ok((
        PublicKey {
            generation,
            public_key: public_key.into(),
        },
        private,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE: &str = "18bbd29f20da4156a9f962c6056f9784";
    const OTHER: &str = "f729ab7824f1486f8587158955e75777";
    const PASSWORD: &[u8] = b"public-test-passphrase";

    #[test]
    fn pbkdf2_matches_independent_reference() {
        let salt = (0u8..16).collect::<Vec<_>>();
        let derived = derive_key(PASSWORD, &salt).unwrap();
        assert_eq!(
            hex(derived.bytes()),
            "298c933c63191cc8cdbf003f93d980554bad3311d4ae095c3f27c80904f13ce4"
        );
    }

    #[test]
    fn envelope_round_trip_encrypts_key_and_authenticates_complete_header() {
        let private = generate().unwrap();
        let public = PublicKey {
            generation: 7,
            public_key: public_coordinate(&private.0).unwrap(),
        };
        let first = encrypt(WORKSPACE, &public, PASSWORD, &private.0).unwrap();
        let second = encrypt(WORKSPACE, &public, PASSWORD, &private.0).unwrap();
        assert_ne!(
            &first[SALT_START..HEADER_BYTES],
            &second[SALT_START..HEADER_BYTES]
        );
        assert!(!first.windows(32).any(|window| window == &private.0[72..]));
        let (actual, recovered) = decrypt(WORKSPACE, &first, PASSWORD).unwrap();
        assert_eq!(actual, public);
        assert!(recovered.bytes() == private.0);
        let reject = |workspace, bytes: &[u8], password: &[u8]| {
            decrypt(workspace, bytes, password)
                .err()
                .expect("expected rejection")
        };
        assert_eq!(
            reject(WORKSPACE, &first, b"wrong-passphrase"),
            "RecoveryBackupAuthenticationFailed"
        );
        assert_eq!(
            reject(OTHER, &first, PASSWORD),
            "RecoveryBackupWorkspaceMismatch"
        );
        assert_eq!(
            reject(WORKSPACE, &first[..first.len() - 1], PASSWORD),
            "RecoveryBackupInvalidHeader"
        );
        for offset in [
            WORKSPACE_START,
            PUBLIC_START - 1,
            SALT_START,
            NONCE_START,
            HEADER_BYTES,
            TAG_START,
        ] {
            let mut modified = first.clone();
            modified[offset] ^= 1;
            assert_eq!(
                reject(WORKSPACE, &modified, PASSWORD),
                "RecoveryBackupAuthenticationFailed"
            );
        }
        let mut modified = first;
        modified[PUBLIC_START] = if modified[PUBLIC_START] == b'0' {
            b'1'
        } else {
            b'0'
        };
        assert_eq!(
            reject(WORKSPACE, &modified, PASSWORD),
            "RecoveryBackupAuthenticationFailed"
        );
    }

    #[test]
    fn refuses_authentic_envelope_with_inconsistent_asymmetric_keypair() {
        let mut private = generate().unwrap();
        let other = generate().unwrap();
        let public = PublicKey {
            generation: 1,
            public_key: public_coordinate(&private.0).unwrap(),
        };
        private.0[72..].copy_from_slice(&other.0[72..]);
        let fake = encrypt(WORKSPACE, &public, PASSWORD, &private.0).unwrap();
        assert!(decrypt(WORKSPACE, &fake, PASSWORD).is_err());
    }
}
