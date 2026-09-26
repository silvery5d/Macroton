use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

pub const MARKER: &str = ".macroton-steam-session";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub app_id: u32,
    pub profile_sha256: String,
    pub runtime_id: String,
    pub runtime_fingerprint_sha256: String,
    pub steam_client_mode: String,
    pub renderer: String,
    pub graphics_api: String,
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn record(prefix: &Path, identity: &Identity) -> Result<(), Error> {
    validate_identity(identity)?;
    let marker = prefix.join(MARKER);
    let temporary = prefix.join(format!("{MARKER}.tmp-{}", std::process::id()));
    let contents = format!(
        concat!(
            "schema=1\n",
            "appid={}\n",
            "profile_sha256={}\n",
            "runtime_id={}\n",
            "runtime_fingerprint_sha256={}\n",
            "steam_client_mode={}\n",
            "renderer={}\n",
            "graphics_api={}\n"
        ),
        identity.app_id,
        identity.profile_sha256,
        identity.runtime_id,
        identity.runtime_fingerprint_sha256,
        identity.steam_client_mode,
        identity.renderer,
        identity.graphics_api,
    );
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!(
            "cannot write temporary Steam session provenance {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, &marker).map_err(|error| Error {
        message: format!(
            "cannot commit Steam session provenance {}: {error}",
            marker.display()
        ),
    })
}

pub fn read(prefix: &Path) -> Result<Identity, Error> {
    let marker = prefix.join(MARKER);
    let contents = fs::read_to_string(&marker).map_err(|error| Error {
        message: format!(
            "cannot read Steam session provenance {}: {error}",
            marker.display()
        ),
    })?;
    parse(&contents)
}

pub fn validate(prefix: &Path, expected: &Identity) -> Result<(), Error> {
    validate_identity(expected)?;
    let actual = read(prefix)?;
    if actual != *expected {
        return Err(Error {
            message: "active Steam session provenance does not match the selected profile/runtime"
                .to_owned(),
        });
    }
    Ok(())
}

pub fn clear(prefix: &Path) -> Result<(), Error> {
    let marker = prefix.join(MARKER);
    match fs::remove_file(&marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error {
            message: format!(
                "cannot clear Steam session provenance {}: {error}",
                marker.display()
            ),
        }),
    }
}

fn parse(contents: &str) -> Result<Identity, Error> {
    let expected_keys = [
        "schema",
        "appid",
        "profile_sha256",
        "runtime_id",
        "runtime_fingerprint_sha256",
        "steam_client_mode",
        "renderer",
        "graphics_api",
    ];
    let mut fields = BTreeMap::new();
    for line in contents.lines() {
        let (key, value) = line.split_once('=').ok_or_else(|| Error {
            message: "Steam session provenance contains a malformed line".to_owned(),
        })?;
        if !expected_keys.contains(&key) {
            return Err(Error {
                message: format!("Steam session provenance contains unknown field {key:?}"),
            });
        }
        if fields.insert(key, value).is_some() {
            return Err(Error {
                message: format!("Steam session provenance contains duplicate field {key:?}"),
            });
        }
    }
    if fields.len() != expected_keys.len() || fields.get("schema") != Some(&"1") {
        return Err(Error {
            message: "Steam session provenance has missing fields or an unsupported schema"
                .to_owned(),
        });
    }
    let identity = Identity {
        app_id: fields["appid"].parse().map_err(|_| Error {
            message: "Steam session provenance has an invalid AppID".to_owned(),
        })?,
        profile_sha256: fields["profile_sha256"].to_owned(),
        runtime_id: fields["runtime_id"].to_owned(),
        runtime_fingerprint_sha256: fields["runtime_fingerprint_sha256"].to_owned(),
        steam_client_mode: fields["steam_client_mode"].to_owned(),
        renderer: fields["renderer"].to_owned(),
        graphics_api: fields["graphics_api"].to_owned(),
    };
    validate_identity(&identity)?;
    Ok(identity)
}

fn validate_identity(identity: &Identity) -> Result<(), Error> {
    if !is_sha256(&identity.profile_sha256) || !is_sha256(&identity.runtime_fingerprint_sha256) {
        return Err(Error {
            message: "Steam session provenance contains an invalid SHA-256".to_owned(),
        });
    }
    for (name, value) in [
        ("runtime_id", identity.runtime_id.as_str()),
        ("steam_client_mode", identity.steam_client_mode.as_str()),
        ("renderer", identity.renderer.as_str()),
        ("graphics_api", identity.graphics_api.as_str()),
    ] {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(Error {
                message: format!("Steam session provenance has invalid {name}"),
            });
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn identity() -> Identity {
        Identity {
            app_id: 42,
            profile_sha256: "ab".repeat(32),
            runtime_id: "wine-test".to_owned(),
            runtime_fingerprint_sha256: "cd".repeat(32),
            steam_client_mode: "standard".to_owned(),
            renderer: "wined3d-opengl".to_owned(),
            graphics_api: "d3d9".to_owned(),
        }
    }

    #[test]
    fn roundtrip_is_strict_and_clear_is_idempotent() {
        let directory = std::env::temp_dir().join(format!(
            "macroton-steam-session-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        let expected = identity();
        record(&directory, &expected).unwrap();
        assert_eq!(read(&directory).unwrap(), expected);
        clear(&directory).unwrap();
        clear(&directory).unwrap();
        assert!(!directory.join(MARKER).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_unknown_duplicate_and_drifted_fields() {
        let valid = format!(
            "schema=1\nappid=42\nprofile_sha256={}\nruntime_id=wine-test\nruntime_fingerprint_sha256={}\nsteam_client_mode=standard\nrenderer=wined3d-opengl\ngraphics_api=d3d9\n",
            "ab".repeat(32),
            "cd".repeat(32)
        );
        assert!(parse(&(valid.clone() + "extra=value\n")).is_err());
        assert!(parse(&(valid.clone() + "appid=42\n")).is_err());
        let drifted = valid.replace("steam_client_mode=standard", "steam_client_mode=other");
        assert_ne!(parse(&drifted).unwrap(), identity());
    }
}
