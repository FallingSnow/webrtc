#![warn(rust_2018_idioms)]
#![allow(dead_code)]

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate serde_derive;

pub mod alert;
pub mod application_data;
pub mod change_cipher_spec;
pub mod cipher_suite;
pub mod client_certificate_type;
pub mod compression_methods;
pub mod config;
pub mod conn;
pub mod content;
pub mod crypto;
pub mod curve;
pub mod errors;
pub mod extension;
pub mod flight;
pub mod fragment_buffer;
pub mod handshake;
pub mod handshaker;
pub mod prf;
pub mod record_layer;
pub mod signature_hash_algorithm;
pub mod state;

use cipher_suite::*;
use extension::extension_use_srtp::SRTPProtectionProfile;

pub(crate) fn find_matching_srtp_profile(
    a: &[SRTPProtectionProfile],
    b: &[SRTPProtectionProfile],
) -> Result<SRTPProtectionProfile, ()> {
    for a_profile in a {
        for b_profile in b {
            if a_profile == b_profile {
                return Ok(*a_profile);
            }
        }
    }
    Err(())
}

pub(crate) fn find_matching_cipher_suite(
    a: &[CipherSuiteID],
    b: &[CipherSuiteID],
) -> Result<CipherSuiteID, ()> {
    for a_suite in a {
        for b_suite in b {
            if a_suite == b_suite {
                return Ok(*a_suite);
            }
        }
    }
    Err(())
}
