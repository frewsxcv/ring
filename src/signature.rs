// Copyright 2015-2016 Brian Smith.
//
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND AND THE AUTHORS DISCLAIM ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY
// SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
// CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

//! Public key signatures: signing and verification.
//!
//! Use the `verify` function to verify signatures, passing a reference to the
//! `_VERIFY` algorithm that identifies the algorithm. See the documentation
//! for `verify` for examples.
//!
//! The design of this module is unusual compared to other public key signature
//! APIs. Algorithms are split into "signing" (suffixed `_SIGN`) and
//! "verification" (suffixed `_VERIFY`) algorithms in order to make it easier
//! for the linker to discard unused code in the case where only signing is
//! done or only verification is done with a given algorithm. (Currently, only
//! the verification algorithms are )
//!
//! Also, this API treats each combination of parameters as a separate
//! algorithm. For example, instead of having a single "RSA" algorithm with a
//! verification function that takes a bunch of parameters, there are
//! `RSA_PKCS1_2048_8192_SHA256_VERIFY`, `RSA_PKCS1_2048_8192_SHA256_VERIFY`,
//! etc. which encode sets of parameter choices into objects. This is designed
//! to reduce the risks of algorithm agility and to provide consistency with
//! ECDSA and EdDSA.
//!
//! Currently this module does not support digesting the message to be signed
//! separately from the public key operation, as it is currently being
//! optimized for Ed25519 and for the implementation of protocols that do not
//! requiring signing large messages. An interface for efficiently supporting
//! larger messages may be added later.

#![allow(unsafe_code)]

use super::{c, bssl};
#[cfg(not(feature = "no_heap"))] use super::{digest, ecc};
use super::input::Input;

/// A signature verification algorithm.
//
// The `VerificationAlgorithm` struct is just a wrapper around a
// `VerificationAlgorithmImpl`. This is done to be consistent with the rest of
// *ring*, which avoids exposing traits in its API, and to save users from
// encountering errors such as:
//
// ```output
// the trait `core::marker::Sync` is not implemented for the type
// `signature::VerificationAlgorithm + 'static` [E0277]
// note: shared static variables must have a type that implements `Sync`
// ```
//
// Although users could resolve such errors by adding `+ Sync` as we do here,
// it's confusing and hard to debug for newcomers.
pub struct VerificationAlgorithm {
    implementation: &'static (VerificationAlgorithmImpl + Sync),
}

trait VerificationAlgorithmImpl {
    fn verify(&self, public_key: Input, msg: Input, signature: Input)
              -> Result<(), ()>;
}

/// Verify the signature `signature` of message `msg` with the public key
/// `public_key` using the algorithm `alg`.
///
/// # Examples
///
/// ## Verify a RSA PKCS#1 signature that uses the SHA-256 digest
///
/// ```ignore
/// # // XXX: Re-enable when https://github.com/rust-lang/rust/pull/30372
/// # // reaches stable.
/// #
/// use ring::input::Input;
/// use ring::signature;
///
/// // Ideally this function should take its inputs as `Input`s instead of
/// // slices. It takes its input as slices to illustrate how to convert slices
/// // to `Input`s.
/// # #[cfg(not(feature = "no_heap"))]
/// fn verify_rsa_pkcs1_sha256(public_key: &[u8], msg: &[u8], sig: &[u8])
///                            -> Result<(), ()> {
///    let public_key = try!(Input::new(public_key));
///    let msg = try!(Input::new(msg));
///    let sig = try!(Input::new(sig));
///    signature::verify(&signature::RSA_PKCS1_2048_8192_SHA256_VERIFY,
///                      public_key, msg, sig)
/// }
/// ```
///
/// ## Verify an Ed25519 signature
///
/// ```
/// use ring::input::Input;
/// use ring::signature;
///
/// fn verify_ed25519(public_key: Input, msg: Input, sig: Input)
///                   -> Result<(), ()> {
///    signature::verify(&signature::ED25519_VERIFY, public_key, msg, sig)
/// }
/// ```
pub fn verify(alg: &VerificationAlgorithm, public_key: Input, msg: Input,
              signature: Input) -> Result<(), ()> {
    alg.implementation.verify(public_key, msg, signature)
}


/// ECDSA Signatures.
#[cfg(not(feature = "no_heap"))]
struct ECDSA {
    digest_alg: &'static digest::Algorithm,
    ec_group_fn: unsafe extern fn() -> *const ecc::EC_GROUP,
}

#[cfg(not(feature = "no_heap"))]
impl VerificationAlgorithmImpl for ECDSA {
    fn verify(&self, public_key: Input, msg: Input, signature: Input)
              -> Result<(), ()> {
        let digest = digest::digest(self.digest_alg, msg.as_slice_less_safe());
        let signature = signature.as_slice_less_safe();
        let public_key = public_key.as_slice_less_safe();
        bssl::map_result(unsafe {
            ECDSA_verify_signed_digest((self.ec_group_fn)(),
                                       digest.algorithm().nid,
                                       digest.as_ref().as_ptr(),
                                       digest.as_ref().len(), signature.as_ptr(),
                                       signature.len(), public_key.as_ptr(),
                                       public_key.len())
        })
    }
}

macro_rules! ecdsa {
    ( $VERIFY_ALGORITHM:ident, $curve_name:expr, $ec_group_fn:expr,
      $digest_alg_name:expr, $digest_alg:expr ) => {
        #[cfg(not(feature = "no_heap"))]
        #[doc="Verification of ECDSA signatures using the "]
        #[doc=$curve_name]
        #[doc=" curve and the "]
        #[doc=$digest_alg_name]
        #[doc=" digest algorithm."]
        ///
        /// Public keys are encoding in uncompressed form using the
        /// Octet-String-to-Elliptic-Curve-Point algorithm in [SEC 1: Elliptic
        /// Curve Cryptography, Version 2.0](http://www.secg.org/sec1-v2.pdf).
        /// Public keys are validated during key agreement according as
        /// described in [NIST Special Publication 800-56A, revision
        /// 2](http://csrc.nist.gov/groups/ST/toolkit/documents/SP800-56Arev1_3-8-07.pdf)
        /// Section 5.6.2.5 and the [Suite B Implementer's Guide to NIST SP
        /// 800-56A](https://www.nsa.gov/ia/_files/suiteb_implementer_g-113808.pdf)
        /// Appendix B.3. Note that, as explained in the NSA guide, "partial"
        /// validation is equivalent to "full" validation for prime-order
        /// curves like this one.
        ///
        /// TODO: Each of the encoded coordinates are verified to be the
        /// correct length, but values of the allowed length that haven't been
        /// reduced modulo *q* are currently reduced mod *q* during
        /// verification. Soon, coordinates larger than *q* - 1 will be
        /// rejected.
        ///
        /// The signature will be parsed as a DER-encoded `Ecdsa-Sig-Value` as
        /// described in [RFC 3279 Section
        /// 2.2.3](https://tools.ietf.org/html/rfc3279#section-2.2.3). Both *r*
        /// and *s* are verified to be in the range [1, *n* - 1].
        ///
        /// Not available in `no_heap` mode.
        pub static $VERIFY_ALGORITHM: VerificationAlgorithm =
                VerificationAlgorithm {
            implementation: &ECDSA {
                digest_alg: $digest_alg,
                ec_group_fn: $ec_group_fn,
            }
        };
    }
}

ecdsa!(ECDSA_P256_SHA1_VERIFY, "P-256 (secp256r1)", ecc::EC_GROUP_P256, "SHA-1",
       &digest::SHA1);
ecdsa!(ECDSA_P256_SHA256_VERIFY, "P-256 (secp256r1)", ecc::EC_GROUP_P256,
       "SHA-256", &digest::SHA256);
ecdsa!(ECDSA_P256_SHA384_VERIFY, "P-256 (secp256r1)", ecc::EC_GROUP_P256,
       "SHA-384", &digest::SHA384);
ecdsa!(ECDSA_P256_SHA512_VERIFY, "P-256 (secp256r1)", ecc::EC_GROUP_P256,
       "SHA-512", &digest::SHA512);

ecdsa!(ECDSA_P384_SHA1_VERIFY, "P-384 (secp384r1)", ecc::EC_GROUP_P384, "SHA-1",
       &digest::SHA1);
ecdsa!(ECDSA_P384_SHA256_VERIFY, "P-384 (secp384r1)", ecc::EC_GROUP_P384,
       "SHA-256", &digest::SHA256);
ecdsa!(ECDSA_P384_SHA384_VERIFY, "P-384 (secp384r1)", ecc::EC_GROUP_P384,
       "SHA-384", &digest::SHA384);
ecdsa!(ECDSA_P384_SHA512_VERIFY, "P-384 (secp384r1)", ecc::EC_GROUP_P384,
       "SHA-512", &digest::SHA512);


/// EdDSA signatures.
struct EdDSA;

/// Verification of [Ed25519](http://ed25519.cr.yp.to/) signatures.
///
/// Ed25519 uses SHA-512 as the digest algorithm.
pub static ED25519_VERIFY: VerificationAlgorithm = VerificationAlgorithm {
    implementation: &EdDSA,
};

#[cfg(test)]
fn ed25519_sign(private_key: &[u8], msg: &[u8], signature: &mut [u8])
                -> Result<(), ()> {
    if private_key.len() != 64 || signature.len() != 64 {
        return Err(());
    }
    bssl::map_result(unsafe {
        ED25519_sign(signature.as_mut_ptr(), msg.as_ptr(), msg.len(),
                     private_key.as_ptr())
    })
}

impl VerificationAlgorithmImpl for EdDSA {
    fn verify(&self, public_key: Input, msg: Input, signature: Input)
              -> Result<(), ()> {
        let public_key = public_key.as_slice_less_safe();
        if public_key.len() != 32 || signature.len() != 64 {
            return Err(())
        }
        let msg = msg.as_slice_less_safe();
        let signature = signature.as_slice_less_safe();
        bssl::map_result(unsafe {
            ED25519_verify(msg.as_ptr(), msg.len(), signature.as_ptr(),
                           public_key.as_ptr())
        })
    }
}


/// RSA PKCS#1 1.5 signatures.
#[allow(non_camel_case_types)]
#[cfg(not(feature = "no_heap"))]
struct RSA_PKCS1 {
    digest_alg: &'static digest::Algorithm,
    min_bits: usize,
}

#[cfg(not(feature = "no_heap"))]
impl VerificationAlgorithmImpl for RSA_PKCS1 {
    fn verify(&self, public_key: Input, msg: Input, signature: Input)
              -> Result<(), ()> {
        let digest = digest::digest(self.digest_alg, msg.as_slice_less_safe());
        let signature = signature.as_slice_less_safe();
        let public_key = public_key.as_slice_less_safe();
        bssl::map_result(unsafe {
            RSA_verify_pkcs1_signed_digest(self.min_bits, 8192,
                                           digest.algorithm().nid,
                                           digest.as_ref().as_ptr(),
                                           digest.as_ref().len(),
                                           signature.as_ptr(), signature.len(),
                                           public_key.as_ptr(), public_key.len())
        })
    }
}

macro_rules! rsa_pkcs1 {
    ( $VERIFY_ALGORITHM:ident, $min_bits:expr, $min_bits_str:expr,
      $digest_alg_name:expr, $digest_alg:expr ) => {
        #[cfg(not(feature = "no_heap"))]
        #[doc="Verification of RSA PKCS#1 1.5 signatures of "]
        #[doc=$min_bits_str]
        #[doc="-8192 bits "]
        #[doc="using the "]
        #[doc=$digest_alg_name]
        #[doc=" digest algorithm."]
        ///
        /// Not available in `no_heap` mode.
        pub static $VERIFY_ALGORITHM: VerificationAlgorithm =
                VerificationAlgorithm {
            implementation: &RSA_PKCS1 {
                digest_alg: $digest_alg,
                min_bits: $min_bits
            }
        };
    }
}

rsa_pkcs1!(RSA_PKCS1_2048_8192_SHA1_VERIFY, 2048, "2048", "SHA-1",
           &digest::SHA1);
rsa_pkcs1!(RSA_PKCS1_2048_8192_SHA256_VERIFY, 2048, "2048", "SHA-256",
           &digest::SHA256);
rsa_pkcs1!(RSA_PKCS1_2048_8192_SHA384_VERIFY, 2048, "2048", "SHA-384",
           &digest::SHA384);
rsa_pkcs1!(RSA_PKCS1_2048_8192_SHA512_VERIFY, 2048, "2048", "SHA-512",
           &digest::SHA512);

rsa_pkcs1!(RSA_PKCS1_3072_8192_SHA384_VERIFY, 3072, "3072", "SHA-384",
           &digest::SHA384);


struct Tmp;

impl VerificationAlgorithmImpl for Tmp {
    fn verify(&self, _public_key: Input, _msg: Input, _signature: Input)
              -> Result<(), ()> {
        unimplemented!()
    }
}

/// RSA_PSS_2048_8192_SHA256
pub static RSA_PSS_2048_8192_SHA256: VerificationAlgorithm = VerificationAlgorithm {
    implementation: &Tmp,
};


extern {
    #[cfg(not(feature = "no_heap"))]
    fn ECDSA_verify_signed_digest(group: *const ecc::EC_GROUP, hash_nid: c::int,
                                  digest: *const u8, digest_len: c::size_t,
                                  sig_der: *const u8, sig_der_len: c::size_t,
                                  key_octets: *const u8,
                                  key_octets_len: c::size_t) -> c::int;

    #[cfg(test)]
    fn ED25519_sign(out_sig: *mut u8/*[64]*/, message: *const u8,
                    message_len: c::size_t, private_key: *const u8/*[64]*/)
                    -> c::int;

    fn ED25519_verify(message: *const u8, message_len: c::size_t,
                      signature: *const u8/*[64]*/,
                      public_key: *const u8/*[32]*/) -> c::int;

    #[cfg(not(feature = "no_heap"))]
    fn RSA_verify_pkcs1_signed_digest(min_bits: usize, max_bits: usize,
                                      digest_nid: c::int, digest: *const u8,
                                      digest_len: c::size_t, sig: *const u8,
                                      sig_len: c::size_t, key_der: *const u8,
                                      key_der_len: c::size_t) -> c::int;
}

#[cfg(test)]
mod tests {
    use super::super::{file_test, signature};
    use super::super::input::Input;

    /// Test vectors from BoringSSL.
    #[test]
    fn test_ed25519() {
        file_test::run("src/ed25519_tests.txt", |section, test_case| {
            assert_eq!(section, "");
            let private_key = test_case.consume_bytes("PRIV");
            assert_eq!(64, private_key.len());
            let public_key = test_case.consume_bytes("PUB");
            assert_eq!(32, public_key.len());
            let msg = test_case.consume_bytes("MESSAGE");
            let expected_sig = test_case.consume_bytes("SIG");

            let mut actual_sig = [0u8; 64];
            assert!(signature::ed25519_sign(&private_key, &msg,
                                            &mut actual_sig).is_ok());
            assert_eq!(&expected_sig[..], &actual_sig[..]);

            let public_key = Input::new(&public_key).unwrap();
            let msg = Input::new(&msg).unwrap();
            let expected_sig = Input::new(&expected_sig).unwrap();

            assert!(signature::verify(&signature::ED25519_VERIFY, public_key,
                                      msg, expected_sig).is_ok());
        });
    }
}
