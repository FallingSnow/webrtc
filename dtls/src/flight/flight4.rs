use super::flight6::*;
use super::*;
use crate::cipher_suite::*;
use crate::client_certificate_type::*;
use crate::compression_methods::*;
use crate::config::*;
use crate::conn::*;
use crate::content::*;
use crate::crypto::*;
use crate::curve::named_curve::*;
use crate::curve::*;
use crate::errors::*;
use crate::extension::extension_supported_elliptic_curves::*;
use crate::extension::extension_supported_point_formats::*;
use crate::extension::extension_use_extended_master_secret::*;
use crate::extension::extension_use_srtp::*;
use crate::extension::*;
use crate::handshake::handshake_header::*;
use crate::handshake::handshake_message_certificate::*;
use crate::handshake::handshake_message_certificate_request::*;
use crate::handshake::handshake_message_server_hello::*;
use crate::handshake::handshake_message_server_hello_done::*;
use crate::handshake::handshake_message_server_key_exchange::*;
use crate::handshake::*;
use crate::prf::*;
use crate::record_layer::record_layer_header::*;
use crate::record_layer::*;
use crate::signature_hash_algorithm::*;

use util::Error;

use std::io::BufWriter;

use async_trait::async_trait;

pub(crate) struct Flight4;

#[async_trait]
impl Flight for Flight4 {
    fn to_string(&self) -> String {
        "Flight4".to_owned()
    }

    async fn parse(
        &self,
        c: &Conn,
        state: &mut State,
        cache: &HandshakeCache,
        cfg: &HandshakeConfig,
    ) -> Result<Box<dyn Flight>, (Option<Alert>, Option<Error>)> {
        let (seq, msgs) = match cache
            .full_pull_map(
                0,
                &[
                    HandshakeCachePullRule {
                        typ: HandshakeType::Certificate,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: true,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ClientKeyExchange,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::CertificateVerify,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: true,
                    },
                ],
            )
            .await
        {
            Ok((seq, msgs)) => (seq, msgs),
            Err(_) => return Err((None, None)),
        };

        let client_key_exchange = if let Some(message) = msgs.get(&HandshakeType::ClientKeyExchange)
        {
            match message {
                HandshakeMessage::ClientKeyExchange(h) => h,
                _ => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InternalError,
                        }),
                        None,
                    ))
                }
            }
        } else {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InternalError,
                }),
                None,
            ));
        };

        if let Some(message) = msgs.get(&HandshakeType::Certificate) {
            let h = match message {
                HandshakeMessage::Certificate(h) => h,
                _ => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InternalError,
                        }),
                        None,
                    ))
                }
            };

            state.peer_certificates = h.certificate.clone();
        }

        if let Some(message) = msgs.get(&HandshakeType::CertificateVerify) {
            let h = match message {
                HandshakeMessage::CertificateVerify(h) => h,
                _ => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InternalError,
                        }),
                        None,
                    ))
                }
            };

            if state.peer_certificates.is_empty() {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::NoCertificate,
                    }),
                    Some(ERR_CERTIFICATE_VERIFY_NO_CERTIFICATE.clone()),
                ));
            }

            let plain_text = cache
                .pull_and_merge(&[
                    HandshakeCachePullRule {
                        typ: HandshakeType::ClientHello,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHello,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::Certificate,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerKeyExchange,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::CertificateRequest,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHelloDone,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::Certificate,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ClientKeyExchange,
                        epoch: cfg.initial_epoch,
                        is_client: true,
                        optional: false,
                    },
                ])
                .await;

            // Verify that the pair of hash algorithm and signature is listed.
            let mut valid_signature_scheme = false;
            for ss in &cfg.local_signature_schemes {
                if ss.hash == h.hash_algorithm && ss.signature == h.signature_algorithm {
                    valid_signature_scheme = true;
                    break;
                }
            }
            if !valid_signature_scheme {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InsufficientSecurity,
                    }),
                    Some(ERR_NO_AVAILABLE_SIGNATURE_SCHEMES.clone()),
                ));
            }

            if let Err(err) = verify_certificate_verify(
                &plain_text,
                /*h.hash_algorithm,*/ &h.signature,
                &state.peer_certificates[0],
            ) {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::BadCertificate,
                    }),
                    Some(err),
                ));
            }

            let mut chains = vec![];
            let mut verified = false;
            if cfg.client_auth as u8 >= ClientAuthType::VerifyClientCertIfGiven as u8 {
                chains = match verify_cert(&state.peer_certificates[0] /*, cfg.clientCAs*/) {
                    Ok(chains) => chains,
                    Err(err) => {
                        return Err((
                            Some(Alert {
                                alert_level: AlertLevel::Fatal,
                                alert_description: AlertDescription::BadCertificate,
                            }),
                            Some(err),
                        ))
                    }
                };
                verified = true
            }
            if let Some(verify_peer_certificate) = &cfg.verify_peer_certificate {
                if let Err(err) = verify_peer_certificate(&state.peer_certificates[0], &chains) {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::BadCertificate,
                        }),
                        Some(err),
                    ));
                }
            }
            state.peer_certificates_verified = verified
        }

        if let Some(cipher_suite) = &state.cipher_suite {
            if !cipher_suite.is_initialized() {
                let mut server_random = vec![];
                {
                    let mut writer = BufWriter::new(server_random.as_mut_slice());
                    let _ = state.local_random.marshal(&mut writer);
                }
                let mut client_random = vec![];
                {
                    let mut writer = BufWriter::new(client_random.as_mut_slice());
                    let _ = state.remote_random.marshal(&mut writer);
                }

                let mut pre_master_secret = vec![];
                if let Some(local_psk_callback) = &cfg.local_psk_callback {
                    let psk = match local_psk_callback(&client_key_exchange.identity_hint) {
                        Ok(psk) => psk,
                        Err(err) => {
                            return Err((
                                Some(Alert {
                                    alert_level: AlertLevel::Fatal,
                                    alert_description: AlertDescription::InternalError,
                                }),
                                Some(err),
                            ))
                        }
                    };

                    pre_master_secret = prf_psk_pre_master_secret(&psk);
                } else if let Some(local_keypair) = &state.local_keypair {
                    pre_master_secret = match prf_pre_master_secret(
                        &client_key_exchange.public_key,
                        &local_keypair.private_key,
                        local_keypair.curve,
                    ) {
                        Ok(pre_master_secret) => pre_master_secret,
                        Err(err) => {
                            return Err((
                                Some(Alert {
                                    alert_level: AlertLevel::Fatal,
                                    alert_description: AlertDescription::IllegalParameter,
                                }),
                                Some(err),
                            ))
                        }
                    };
                }

                if let Some(cipher_suite) = &state.cipher_suite {
                    if state.extended_master_secret {
                        let hf = cipher_suite.hash_func();
                        let session_hash =
                            match cache.session_hash(hf, cfg.initial_epoch, &[]).await {
                                Ok(s) => s,
                                Err(err) => {
                                    return Err((
                                        Some(Alert {
                                            alert_level: AlertLevel::Fatal,
                                            alert_description: AlertDescription::InternalError,
                                        }),
                                        Some(err),
                                    ))
                                }
                            };

                        state.master_secret = match prf_extended_master_secret(
                            &pre_master_secret,
                            &session_hash,
                            cipher_suite.hash_func(),
                        ) {
                            Ok(ms) => ms,
                            Err(err) => {
                                return Err((
                                    Some(Alert {
                                        alert_level: AlertLevel::Fatal,
                                        alert_description: AlertDescription::InternalError,
                                    }),
                                    Some(err),
                                ))
                            }
                        };
                    } else {
                        state.master_secret = match prf_master_secret(
                            &pre_master_secret,
                            &client_random,
                            &server_random,
                            cipher_suite.hash_func(),
                        ) {
                            Ok(ms) => ms,
                            Err(err) => {
                                return Err((
                                    Some(Alert {
                                        alert_level: AlertLevel::Fatal,
                                        alert_description: AlertDescription::InternalError,
                                    }),
                                    Some(err),
                                ))
                            }
                        };
                    }
                }

                if let Some(cipher_suite) = &mut state.cipher_suite {
                    if let Err(err) = cipher_suite.init(
                        &state.master_secret,
                        &client_random,
                        &server_random,
                        false,
                    ) {
                        return Err((
                            Some(Alert {
                                alert_level: AlertLevel::Fatal,
                                alert_description: AlertDescription::InternalError,
                            }),
                            Some(err),
                        ));
                    }
                }
            }
        }

        // Now, encrypted packets can be handled
        if let Err(err) = c.handle_queued_packets() {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InternalError,
                }),
                Some(err),
            ));
        }

        let (seq, msgs) = match cache
            .full_pull_map(
                seq,
                &[HandshakeCachePullRule {
                    typ: HandshakeType::Finished,
                    epoch: cfg.initial_epoch + 1,
                    is_client: true,
                    optional: false,
                }],
            )
            .await
        {
            Ok((seq, msgs)) => (seq, msgs),
            // No valid message received. Keep reading
            Err(_) => return Err((None, None)),
        };

        state.handshake_recv_sequence = seq;

        if let Some(message) = msgs.get(&HandshakeType::Finished) {
            match message {
                HandshakeMessage::Finished(h) => h,
                _ => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InternalError,
                        }),
                        None,
                    ))
                }
            }
        } else {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InternalError,
                }),
                None,
            ));
        };

        match cfg.client_auth {
            ClientAuthType::RequireAnyClientCert => {
                if state.peer_certificates.is_empty() {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::NoCertificate,
                        }),
                        Some(ERR_CLIENT_CERTIFICATE_REQUIRED.clone()),
                    ));
                }
            }
            ClientAuthType::VerifyClientCertIfGiven => {
                if !state.peer_certificates.is_empty() && !state.peer_certificates_verified {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::BadCertificate,
                        }),
                        Some(ERR_CLIENT_CERTIFICATE_NOT_VERIFIED.clone()),
                    ));
                }
            }
            ClientAuthType::RequireAndVerifyClientCert => {
                if state.peer_certificates.is_empty() {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::NoCertificate,
                        }),
                        Some(ERR_CLIENT_CERTIFICATE_REQUIRED.clone()),
                    ));
                }
                if !state.peer_certificates_verified {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::BadCertificate,
                        }),
                        Some(ERR_CLIENT_CERTIFICATE_NOT_VERIFIED.clone()),
                    ));
                }
            }
            ClientAuthType::NoClientCert | ClientAuthType::RequestClientCert => {
                return Ok(Box::new(Flight6 {}));
            }
        }

        Ok(Box::new(Flight6 {}))
    }

    async fn generate(
        &self,
        _c: &Conn,
        state: &mut State,
        _cache: &HandshakeCache,
        cfg: &HandshakeConfig,
    ) -> Result<Vec<Packet>, (Option<Alert>, Option<Error>)> {
        let mut extensions = vec![];
        if (cfg.extended_master_secret == ExtendedMasterSecretType::Request
            || cfg.extended_master_secret == ExtendedMasterSecretType::Require)
            && state.extended_master_secret
        {
            extensions.push(Extension::UseExtendedMasterSecret(
                ExtensionUseExtendedMasterSecret { supported: true },
            ));
        }

        if state.srtp_protection_profile != SRTPProtectionProfile::Unsupported {
            extensions.push(Extension::UseSRTP(ExtensionUseSRTP {
                protection_profiles: vec![state.srtp_protection_profile],
            }));
        }

        if cfg.local_psk_callback.is_none() {
            extensions.extend_from_slice(&[
                Extension::SupportedEllipticCurves(ExtensionSupportedEllipticCurves {
                    elliptic_curves: vec![NamedCurve::X25519, NamedCurve::P256, NamedCurve::P384],
                }),
                Extension::SupportedPointFormats(ExtensionSupportedPointFormats {
                    point_formats: vec![ELLIPTIC_CURVE_POINT_FORMAT_UNCOMPRESSED],
                }),
            ]);
        }

        let mut pkts = vec![Packet {
            record: RecordLayer {
                record_layer_header: RecordLayerHeader {
                    protocol_version: PROTOCOL_VERSION1_2,
                    ..Default::default()
                },
                content: Content::Handshake(Handshake {
                    handshake_header: HandshakeHeader::default(),
                    handshake_message: HandshakeMessage::ServerHello(HandshakeMessageServerHello {
                        version: PROTOCOL_VERSION1_2,
                        random: state.local_random.clone(),
                        cipher_suite: if let Some(cipher_suite) = &state.cipher_suite {
                            cipher_suite.id()
                        } else {
                            CipherSuiteID::Unsupported
                        },
                        compression_method: default_compression_methods().ids[0],
                        extensions,
                    }),
                }),
            },
            should_encrypt: false,
            reset_local_sequence_number: false,
        }];

        if cfg.local_psk_callback.is_none() {
            let certificate = match cfg.get_certificate(&cfg.server_name) {
                Ok(cert) => cert,
                Err(err) => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::HandshakeFailure,
                        }),
                        Some(err),
                    ))
                }
            };

            pkts.push(Packet {
                record: RecordLayer {
                    record_layer_header: RecordLayerHeader {
                        protocol_version: PROTOCOL_VERSION1_2,
                        ..Default::default()
                    },
                    content: Content::Handshake(Handshake {
                        handshake_header: HandshakeHeader::default(),
                        handshake_message: HandshakeMessage::Certificate(
                            HandshakeMessageCertificate {
                                certificate: vec![certificate.certificate.clone()],
                            },
                        ),
                    }),
                },
                should_encrypt: false,
                reset_local_sequence_number: false,
            });

            let mut server_random = vec![];
            {
                let mut writer = BufWriter::new(server_random.as_mut_slice());
                let _ = state.local_random.marshal(&mut writer);
            }
            let mut client_random = vec![];
            {
                let mut writer = BufWriter::new(client_random.as_mut_slice());
                let _ = state.remote_random.marshal(&mut writer);
            }

            // Find compatible signature scheme
            let signature_hash_algo = match select_signature_scheme(
                &cfg.local_signature_schemes,
                &certificate.private_key,
            ) {
                Ok(s) => s,
                Err(err) => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InsufficientSecurity,
                        }),
                        Some(err),
                    ))
                }
            };

            if let Some(local_keypair) = &state.local_keypair {
                let signature = match generate_key_signature(
                    &client_random,
                    &server_random,
                    &local_keypair.public_key,
                    state.named_curve,
                    &certificate.private_key, /*, signature_hash_algo.hash*/
                ) {
                    Ok(s) => s,
                    Err(err) => {
                        return Err((
                            Some(Alert {
                                alert_level: AlertLevel::Fatal,
                                alert_description: AlertDescription::InternalError,
                            }),
                            Some(err),
                        ))
                    }
                };

                state.local_key_signature = signature;

                pkts.push(Packet {
                    record: RecordLayer {
                        record_layer_header: RecordLayerHeader {
                            protocol_version: PROTOCOL_VERSION1_2,
                            ..Default::default()
                        },
                        content: Content::Handshake(Handshake {
                            handshake_header: HandshakeHeader::default(),
                            handshake_message: HandshakeMessage::ServerKeyExchange(
                                HandshakeMessageServerKeyExchange {
                                    identity_hint: vec![],
                                    elliptic_curve_type: EllipticCurveType::NamedCurve,
                                    named_curve: state.named_curve,
                                    public_key: local_keypair.public_key.clone(),
                                    hash_algorithm: signature_hash_algo.hash,
                                    signature_algorithm: signature_hash_algo.signature,
                                    signature: state.local_key_signature.clone(),
                                },
                            ),
                        }),
                    },
                    should_encrypt: false,
                    reset_local_sequence_number: false,
                });
            }

            if cfg.client_auth as u8 > ClientAuthType::NoClientCert as u8 {
                pkts.push(Packet {
                    record: RecordLayer {
                        record_layer_header: RecordLayerHeader {
                            protocol_version: PROTOCOL_VERSION1_2,
                            ..Default::default()
                        },
                        content: Content::Handshake(Handshake {
                            handshake_header: HandshakeHeader::default(),
                            handshake_message: HandshakeMessage::CertificateRequest(
                                HandshakeMessageCertificateRequest {
                                    certificate_types: vec![
                                        ClientCertificateType::RSASign,
                                        ClientCertificateType::ECDSASign,
                                    ],
                                    signature_hash_algorithms: cfg.local_signature_schemes.clone(),
                                },
                            ),
                        }),
                    },
                    should_encrypt: false,
                    reset_local_sequence_number: false,
                });
            }
        } else if !cfg.local_psk_identity_hint.is_empty() {
            // To help the client in selecting which identity to use, the server
            // can provide a "PSK identity hint" in the ServerKeyExchange message.
            // If no hint is provided, the ServerKeyExchange message is omitted.
            //
            // https://tools.ietf.org/html/rfc4279#section-2
            pkts.push(Packet {
                record: RecordLayer {
                    record_layer_header: RecordLayerHeader {
                        protocol_version: PROTOCOL_VERSION1_2,
                        ..Default::default()
                    },
                    content: Content::Handshake(Handshake {
                        handshake_header: HandshakeHeader::default(),
                        handshake_message: HandshakeMessage::ServerKeyExchange(
                            HandshakeMessageServerKeyExchange {
                                identity_hint: cfg.local_psk_identity_hint.clone(),
                                elliptic_curve_type: EllipticCurveType::Unsupported,
                                named_curve: NamedCurve::Unsupported,
                                public_key: vec![],
                                hash_algorithm: HashAlgorithm::Unsupported,
                                signature_algorithm: SignatureAlgorithm::Unsupported,
                                signature: vec![],
                            },
                        ),
                    }),
                },
                should_encrypt: false,
                reset_local_sequence_number: false,
            });
        }

        pkts.push(Packet {
            record: RecordLayer {
                record_layer_header: RecordLayerHeader {
                    protocol_version: PROTOCOL_VERSION1_2,
                    ..Default::default()
                },
                content: Content::Handshake(Handshake {
                    handshake_header: HandshakeHeader::default(),
                    handshake_message: HandshakeMessage::ServerHelloDone(
                        HandshakeMessageServerHelloDone {},
                    ),
                }),
            },
            should_encrypt: false,
            reset_local_sequence_number: false,
        });

        Ok(pkts)
    }
}