use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use rand::random;
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey, EncodeRsaPrivateKey, EncodeRsaPublicKey},
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    pkcs8::LineEnding,
    sha2::{Digest, Sha256},
    signature::Verifier,
    // traits::SignatureScheme,
    Pkcs1v15Encrypt,
    RsaPrivateKey,
    RsaPublicKey,
};

use std::{
    fs::OpenOptions,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct Encrypter(pub RsaPublicKey);

impl Encrypter {
    pub fn create(pub_key: RsaPublicKey) -> Self {
        Encrypter(pub_key)
    }

    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.0.hash(&mut hasher);
        hasher.finish()
    }

    pub fn pub_key_der(&self) -> Vec<u8> {
        self.0.to_pkcs1_der().unwrap().to_vec()
    }
    pub fn create_from_data(data: &str) -> Result<Self, String> {
        let res = DecodeRsaPublicKey::from_pkcs1_pem(data);
        if let Ok(pub_key) = res {
            return Ok(Encrypter::create(pub_key));
        } else {
            println!("Error creating PubRsaKey: {:?}", res);
        }
        Err("Unable to create Encrypter from data".to_string())
    }

    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut rng = rand::thread_rng();
        let res = self.0.encrypt(&mut rng, Pkcs1v15Encrypt, data);
        if let Ok(vector) = res {
            Ok(vector)
        } else {
            println!("Fail: {:?}", res);
            Err("Unable to encrypt data".to_string())
        }
    }
    pub fn verify(&self, data: &[u8], signature: &[u8]) -> bool {
        // println!("OrigSig: {:?}", signature);
        // println!("StrgSig: {:?}", s_string);
        // let signature_two = Signature::try_from(s_string.into_bytes()).unwrap();
        // let mut bytes: Vec<u8> = vec![];
        // for i in (0..s_string.len()).step_by(2) {
        //     bytes.push(u8::from_str_radix(&s_string[i..i + 2], 16).unwrap());
        //     // .collect();
        // }
        let signature_three = Signature::try_from(signature).unwrap();

        // println!("ConvSig: {:?}", signature_two);
        let verifier: VerifyingKey<Sha256> = VerifyingKey::new(self.0.clone());
        // let mut rng = OsRng;
        // let res = verifier.verify(data, &signature).is_ok();
        // println!("Verify result: {:?}", res);
        // let res = verifier.verify(data, &signature_two).is_ok();
        // println!("Verify result: {:?}", res);
        verifier.verify(data, &signature_three).is_ok()
        // println!("Verify result: {:?}", res);
        // res
    }
}
#[derive(Clone)]
pub struct Decrypter(RsaPrivateKey);

impl Decrypter {
    pub fn create(priv_key: RsaPrivateKey) -> Self {
        Decrypter(priv_key)
    }

    // For Multicast/Broadcast messaging, where private key is shared
    // and source uses public key for encryption
    pub fn create_from_data(data: &str) -> Result<Self, String> {
        if let Ok(priv_key) = DecodeRsaPrivateKey::from_pkcs1_pem(data) {
            return Ok(Decrypter::create(priv_key));
        }
        Err("Unable to create Encrypter from data".to_string())
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if let Ok(dec_data) = self.0.decrypt(Pkcs1v15Encrypt, data) {
            Ok(dec_data)
        } else {
            Err("Unable to decrypt data".to_string())
        }
    }

    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ()> {
        let signer: SigningKey<Sha256> = SigningKey::new(self.0.clone());
        // let mut rng = OsRng;
        let mut digest = <Sha256 as Digest>::new();
        digest.update(data);
        use rsa::signature::DigestSigner;
        // let signature_result = signer.sign_digest(digest);
        let signature_result = signer.sign_digest(digest);
        let signature_string = signature_result.to_string();
        let mut bytes: Vec<u8> = vec![];
        for i in (0..signature_string.len()).step_by(2) {
            bytes.push(u8::from_str_radix(&signature_string[i..i + 2], 16).unwrap());
        }
        Ok(bytes)
        // Ok(signature_result)
        // if let Ok(signature) = signature_result {
        //     Ok(signature)
        // } else {
        //     println!("Unable to sign: {:?}", signature_result);
        //     Err(())
        // }
    }
}

pub struct SessionKey(Aes256Gcm);

impl SessionKey {
    pub fn from_key(key_str: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key_str);
        Self(Aes256Gcm::new(key))
    }
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        // fn aes_encrypt(key_str: String, plaintext: String) -> Vec<u8> {
        // let key = Aes256Gcm::generate_key(&mut OsRng);
        // let key = Key::<Aes256Gcm>::from_slice(key_str.as_bytes());
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

        let ciphered_data = self
            .0
            .encrypt(&nonce, plaintext)
            .expect("failed to encrypt with session key");

        // combining nonce and encrypted data together
        // for storage purpose
        let mut encrypted_data: Vec<u8> = nonce.to_vec();
        encrypted_data.extend_from_slice(&ciphered_data);

        encrypted_data
    }

    pub fn decrypt(&self, encrypted_data: &[u8]) -> Result<Vec<u8>, String> {
        // let key = Key::<Aes256Gcm>::from_slice(&key_str);

        // println!("Got: {:?} {}", encrypted_data, encrypted_data.len());
        let (nonce_arr, ciphered_data) = encrypted_data.split_at(12);
        let nonce = Nonce::from_slice(nonce_arr);
        let res = self.0.decrypt(nonce, ciphered_data);
        if let Ok(data) = res {
            // println!("ED: {:?}", encrypted_data);
            Ok(data)
        } else {
            // println!("ED: {:?}", encrypted_data);
            // println!("Err: {:?}", res.err().unwrap());
            Err("Failed to decrypt data".to_string())
        }
    }
}

pub fn generate_symmetric_key() -> [u8; 32] {
    let mut resulting_array: [u8; 32] = [0; 32];
    for byte in &mut resulting_array {
        *byte = random::<u8>()
    }
    resulting_array
}
pub fn get_key_pair_from_files(
    priv_path: PathBuf,
    pub_path: PathBuf,
) -> Option<(RsaPrivateKey, RsaPublicKey)> {
    if let Ok(priv_key) = DecodeRsaPrivateKey::read_pkcs1_pem_file(priv_path) {
        if let Ok(pub_key) = DecodeRsaPublicKey::read_pkcs1_pem_file(pub_path) {
            return Some((priv_key, pub_key));
        }
    }
    None
}

pub fn store_key_pair_as_pem_files(
    priv_key: &RsaPrivateKey,
    pub_key: &RsaPublicKey,
    folder: PathBuf,
) -> Result<(), String> {
    let priv_path = folder.join("id_rsa");
    println!("priv: {:?}", priv_path);
    if !priv_path.exists() {
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .append(true)
            .open(priv_path.clone())
            .unwrap();
    }
    let result_prv =
        EncodeRsaPrivateKey::write_pkcs1_pem_file(priv_key, priv_path, LineEnding::default());
    if result_prv.is_ok() {
        let pub_path = folder.join("id_rsa.pub");
        if !pub_path.exists() {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .append(true)
                .open(pub_path.clone())
                .unwrap();
        }
        let result_pub =
            EncodeRsaPublicKey::write_pkcs1_pem_file(pub_key, pub_path, LineEnding::default());
        if result_pub.is_ok() {
            Ok(())
        } else {
            Err("Could not write pub file".to_string())
        }
    } else {
        Err("Could not write priv file".to_string())
    }
}

pub fn get_new_key_pair(bits: usize) -> Option<(RsaPrivateKey, RsaPublicKey)> {
    let mut rng = rand::thread_rng();
    // let bits = 2048;
    if let Ok(priv_key) = RsaPrivateKey::new(&mut rng, bits) {
        let pub_key = RsaPublicKey::from(&priv_key);
        Some((priv_key, pub_key))
    } else {
        None
    }
}
