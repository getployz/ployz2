//! Docker Volume plugin removal and lookup endpoints.

use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;

use super::{DockerVolumeName, ErrorResponse, VolumeRequest, VolumeStorage, error_response};

/// Destroys the requested Provisioned Volume dataset when it exists.
pub(super) async fn remove(
    State(storage): State<VolumeStorage>,
    Json(request): Json<VolumeRequest>,
) -> Json<ErrorResponse> {
    let result = match request.name.parse::<DockerVolumeName>() {
        Ok(name) => storage.remove(&name).await,
        Err(error) => Err(error),
    };
    error_response(result)
}

/// Returns the requested Provisioned Volume to Docker when it exists.
pub(super) async fn get(
    State(storage): State<VolumeStorage>,
    Json(request): Json<VolumeRequest>,
) -> impl IntoResponse {
    let result = match request.name.parse::<DockerVolumeName>() {
        Ok(name) => storage.inspect(&name).await.map(|mountpoint| PluginVolume {
            name: name.to_string(),
            mountpoint,
        }),
        Err(error) => Err(error),
    };
    Json(match result {
        Ok(volume) => GetResponse::Found { volume, error: () },
        Err(error) => GetResponse::Error {
            error: error.to_string(),
        },
    })
}

#[derive(Serialize)]
#[serde(untagged)]
enum GetResponse {
    Found {
        #[serde(rename = "Volume")]
        volume: PluginVolume,
        #[serde(rename = "Err", serialize_with = "empty_error")]
        error: (),
    },
    Error {
        #[serde(rename = "Err")]
        error: String,
    },
}

fn empty_error<S>((): &(), serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str("")
}

#[derive(Serialize)]
struct PluginVolume {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Mountpoint")]
    mountpoint: String,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tokio::net::UnixListener;

    use super::super::{
        VolumeStorage, serve,
        tests::{TestDir, error, fake_zfs, post},
    };

    #[tokio::test]
    async fn remove_is_exact_idempotent_and_reports_destroy_failures() {
        let test = TestDir::new();
        for marker in ["root", "volume", "sibling"] {
            fs::write(test.0.join(marker), "").unwrap();
        }
        let (zpool, zfs) = fake_zfs(&test.0, "tank\tONLINE\toff\n");
        let socket = test.0.join("plugin.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(serve(listener, VolumeStorage::with_programs(zpool, zfs)));

        assert_eq!(
            post(&socket, "/VolumeDriver.Get", json!({"Name":"data"})).await,
            json!({"Volume":{"Name":"data","Mountpoint":"/var/lib/ployz-volumes/data"},"Err":""})
        );
        assert_eq!(
            post(&socket, "/VolumeDriver.Remove", json!({"Name":"missing"})).await,
            json!({"Err":""})
        );
        assert!(
            !error(&post(&socket, "/VolumeDriver.Remove", json!({"Name":"../data"})).await)
                .is_empty()
        );
        fs::write(test.0.join("fail-destroy"), "").unwrap();
        assert!(
            error(&post(&socket, "/VolumeDriver.Remove", json!({"Name":"data"})).await)
                .contains("destroy failed")
        );
        fs::remove_file(test.0.join("fail-destroy")).unwrap();
        assert_eq!(
            post(&socket, "/VolumeDriver.Remove", json!({"Name":"data"})).await,
            json!({"Err":""})
        );
        assert_eq!(
            post(&socket, "/VolumeDriver.Remove", json!({"Name":"data"})).await,
            json!({"Err":""})
        );

        let log = fs::read_to_string(test.0.join("commands")).unwrap();
        assert_eq!(log.matches("zfs destroy tank/ployz/data").count(), 2);
        assert!(!log.contains("destroy tank/ployz/sibling"));
        assert!(test.0.join("sibling").exists());
        server.abort();
    }
}
