//! Apple iCloud SRP (2048-bit, SHA-256, username omitted from inner hash).
//! Ported from [icloud-reminders-cli/internal/srp](https://github.com/tarekbecker/icloud-reminders-cli).

use num_bigint::BigUint;
use num_traits::Num;
use rand::RngCore;
use sha2::{Digest, Sha256};

const N_HEX_2048: &str = "
AC6BDB41 324A9A9B F166DE5E 1389582F AF72B665 1987EE07 FC319294
3DB56050 A37329CB B4A099ED 8193E075 7767A13D D52312AB 4B03310D
CD7F48A9 DA04FD50 E8083969 EDB767B0 CF609517 9A163AB3 661A05FB
D5FAAAE8 2918A996 2F0B93B8 55F97993 EC975EEA A80D740A DBF4FF74
7359D041 D5C33EA7 1D281E44 6B14773B CA97B43A 23FB8016 76BD207A
436C6481 F1D2B907 8717461A 5B9D32E6 88F87748 544523B5 24B0D57D
5EA77A27 75D2ECFA 032CFBDB F52FB378 61602790 04E57AE6 AF874E73
03CE5329 9CCC041C 7BC308D8 2A5698F3 A8D0C382 71AE35F8 E9DBFBB6
94B5C803 D89F7AE4 35DE236D 525F5475 9B65E372 FCD68EF2 0FA7111F
9E4AFF73
";

#[derive(Clone)]
pub struct SrpParams {
    pub g: BigUint,
    pub n: BigUint,
    pub n_bits: usize,
    pub no_username_in_x: bool,
}

impl SrpParams {
    pub fn apple_2048() -> Self {
        let hex: String = N_HEX_2048.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let n = BigUint::from_str_radix(&hex, 16).expect("N hex");
        Self {
            g: BigUint::from(2u8),
            n,
            n_bits: 2048,
            no_username_in_x: true,
        }
    }

    pub(crate) fn n_len_bytes(&self) -> usize {
        self.n_bits / 8
    }

    fn pad_to_n_uint(&self, v: &BigUint) -> Vec<u8> {
        pad_to_fixed(&v.to_bytes_be(), self.n_len_bytes())
    }

    pub(crate) fn get_multiplier(&self) -> BigUint {
        let mut h = Sha256::new();
        let n_bytes = pad_to_fixed(&self.n.to_bytes_be(), self.n_len_bytes());
        let mut g_bytes = self.g.to_bytes_be();
        while g_bytes.len() < n_bytes.len() {
            g_bytes.insert(0, 0);
        }
        h.update(&n_bytes);
        h.update(&g_bytes);
        BigUint::from_bytes_be(&h.finalize())
    }

    pub(crate) fn calculate_a(&self, a: &BigUint) -> Vec<u8> {
        let an = self.g.modpow(a, &self.n);
        self.pad_to_n_uint(&an)
    }

    pub(crate) fn calculate_u(&self, aa: &[u8], bb: &[u8]) -> BigUint {
        let aa_p = pad_to_fixed(aa, self.n_len_bytes());
        let bb_p = pad_to_fixed(bb, self.n_len_bytes());
        let mut h = Sha256::new();
        h.update(&aa_p);
        h.update(&bb_p);
        BigUint::from_bytes_be(&h.finalize())
    }

    pub(crate) fn calculate_x(&self, salt: &[u8], username: &[u8], p: &[u8]) -> BigUint {
        let mut h1 = Sha256::new();
        if !self.no_username_in_x {
            h1.update(username);
        }
        h1.update(b":");
        h1.update(p);
        let inner = h1.finalize();

        let mut h2 = Sha256::new();
        h2.update(salt);
        h2.update(inner);
        BigUint::from_bytes_be(&h2.finalize())
    }

    /// Client-side `S = (B - k*g^x)^(a + u*x) mod N` (matches Go `calculateS`).
    pub(crate) fn calculate_s(
        &self,
        k: &BigUint,
        x: &BigUint,
        a_secret: &BigUint,
        b: &BigUint,
        u: &BigUint,
    ) -> Vec<u8> {
        let gx = self.g.modpow(x, &self.n);
        let kgx = (k * gx) % &self.n;
        let b_mod = b % self.n.clone();
        let base = (b_mod + &self.n - kgx) % &self.n;
        let exp = a_secret + u * x;
        let s = base.modpow(&exp, &self.n);
        self.pad_to_n_uint(&s)
    }

    pub(crate) fn calculate_k(&self, s: &[u8]) -> Vec<u8> {
        Sha256::digest(s).to_vec()
    }

    pub(crate) fn calculate_m1(&self, username: &[u8], salt: &[u8], a: &[u8], b: &[u8], kk: &[u8]) -> Vec<u8> {
        let digest_g = Sha256::digest(self.pad_to_n_uint(&self.g));
        let digest_n = Sha256::digest(self.n.to_bytes_be());
        let digest_i = Sha256::digest(username);
        let mut hxor = vec![0u8; 32];
        for i in 0..32 {
            hxor[i] = digest_n[i] ^ digest_g[i];
        }
        let a_p = pad_to_fixed(a, self.n_len_bytes());
        let b_p = pad_to_fixed(b, self.n_len_bytes());
        let mut h = Sha256::new();
        h.update(&hxor);
        h.update(digest_i);
        h.update(salt);
        h.update(&a_p);
        h.update(&b_p);
        h.update(kk);
        h.finalize().to_vec()
    }

    pub(crate) fn calculate_m2(&self, a: &[u8], m1: &[u8], kk: &[u8]) -> Vec<u8> {
        let a_p = pad_to_fixed(a, self.n_len_bytes());
        let mut h = Sha256::new();
        h.update(&a_p);
        h.update(m1);
        h.update(kk);
        h.finalize().to_vec()
    }
}

fn pad_to_fixed(bytes: &[u8], length: usize) -> Vec<u8> {
    if bytes.len() >= length {
        bytes[bytes.len() - length..].to_vec()
    } else {
        let mut v = vec![0u8; length - bytes.len()];
        v.extend_from_slice(bytes);
        v
    }
}

pub struct SrpClient {
    params: SrpParams,
    secret_a: BigUint,
    pub aa: Vec<u8>,
    m1: Vec<u8>,
    m2: Vec<u8>,
}

impl SrpClient {
    pub fn new(params: SrpParams, seed: Option<[u8; 32]>) -> Self {
        let mut sec = [0u8; 32];
        if let Some(s) = seed {
            sec = s;
        } else {
            rand::thread_rng().fill_bytes(&mut sec);
        }
        let secret_a = BigUint::from_bytes_be(&sec);
        let aa = params.calculate_a(&secret_a);
        Self {
            params,
            secret_a,
            aa,
            m1: vec![],
            m2: vec![],
        }
    }

    pub fn a_b64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.aa)
    }

    pub fn process_challenge(&mut self, username: &str, pass_key: &[u8], salt: &[u8], b_bytes: &[u8]) {
        let x = self
            .params
            .calculate_x(salt, username.as_bytes(), pass_key);
        let big_b = BigUint::from_bytes_be(b_bytes);
        let u = self.params.calculate_u(&self.aa, b_bytes);
        let k = self.params.get_multiplier();
        let s = self
            .params
            .calculate_s(&k, &x, &self.secret_a, &big_b, &u);
        let key = self.params.calculate_k(&s);
        self.m1 = self
            .params
            .calculate_m1(username.as_bytes(), salt, &self.aa, b_bytes, &key);
        self.m2 = self.params.calculate_m2(&self.aa, &self.m1, &key);
    }

    pub fn m1_b64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.m1)
    }

    pub fn m2_b64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.m2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_and_group() {
        let p = SrpParams::apple_2048();
        assert_eq!(p.n_len_bytes(), 256);
    }
}
