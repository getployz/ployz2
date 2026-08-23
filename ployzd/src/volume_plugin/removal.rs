use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;

use super::{
    DATASET_ROOT, Dataset, DockerVolumeName, ErrorResponse, Result, VolumeRequest, VolumeStorage,
    error_response,
};

impl VolumeStorage {
    async fn requested_dataset(&self, name: &DockerVolumeName) -> Result<Option<Dataset>> {
        let pool = self.one_pool().await?;
        let requested = format!("{}/{DATASET_ROOT}/{name}", pool.name);
        Ok(self
            .datasets(&pool)
            .await?
            .into_iter()
            .find(|dataset| dataset.name == requested))
    }

    async fn remove(&self, name: &DockerVolumeName) -> Result<()> {
        let _guard = self.mutation.lock().await;
        if let Some(dataset) = self.requested_dataset(name).await? {
            self.zfs(&["destroy", &dataset.name]).await?;
        }
        Ok(())
    }

    async fn inspect(&self, name: &DockerVolumeName) -> Result<String> {
        let _guard = self.mutation.lock().await;
        let dataset = self
            .requested_dataset(name)
            .await?
            .ok_or_else(|| format!("Provisioned Volume {name} does not exist"))?;
        dataset.require_mountpoint(&name.mountpoint())?;
        Ok(dataset.mountpoint)
    }
}

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
    let (volume, error) = match result {
        Ok(volume) => (Some(volume), String::new()),
        Err(error) => (None, error.to_string()),
    };
    Json(GetResponse { volume, error })
}

#[derive(Serialize)]
struct GetResponse {
    #[serde(rename = "Volume", skip_serializing_if = "Option::is_none")]
    volume: Option<PluginVolume>,
    #[serde(rename = "Err")]
    error: String,
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
