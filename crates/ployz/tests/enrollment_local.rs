use ployz::enrollment::local::{has_assignment, save_assignment};
use ployz_core::{
    AdvertisedEndpoint, EnrollmentAssignment, EnrollmentSnapshot, MachineId, MachineName,
    RegisterRequest, StorageChoice, WireGuardPublicKey, allocate_enrollment,
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn request(seed: u8) -> RegisterRequest {
    RegisterRequest {
        machine_id: Some(MachineId::parse(format!("{seed:032x}")).unwrap()),
        assigned_subnet: None,
        initial_policy: Default::default(),
        name: MachineName::parse("same-name").unwrap(),
        storage: StorageChoice::None,
        public_key: WireGuardPublicKey([seed; 32]),
        public_ip: None,
        advertised_endpoints: vec![AdvertisedEndpoint("192.0.2.1:51820".parse().unwrap())],
        runtime: Default::default(),
    }
}

fn snapshot(seed: u8) -> EnrollmentSnapshot {
    let mut snapshot = EnrollmentSnapshot {
        network: "10.210.0.0/22".parse().unwrap(),
        machines: vec![],
        target_versions: Default::default(),
    };
    snapshot.machines.push(
        allocate_enrollment(&request(seed), &snapshot, &[])
            .unwrap()
            .machine,
    );
    snapshot
}

#[test]
fn enrollment_process_worker() {
    let Ok(directory) = std::env::var("PLOYZ_TEST_ENROLLMENT_DIRECTORY") else {
        return;
    };
    let seed: u8 = std::env::var("PLOYZ_TEST_ENROLLMENT_SEED")
        .unwrap()
        .parse()
        .unwrap();
    let result_path = std::env::var("PLOYZ_TEST_ENROLLMENT_RESULT").unwrap();
    let snapshot = snapshot(1); // every independent command holds the same stale observation
    fs::write(format!("{result_path}.ready"), []).unwrap();
    let assignment = save_assignment(Path::new(&directory), &request(seed), &snapshot).unwrap();
    fs::write(result_path, serde_json::to_vec(&assignment).unwrap()).unwrap();
}

fn concurrent(directory: &Path, seeds: &[u8]) -> Vec<EnrollmentAssignment> {
    fs::create_dir_all(directory).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("lock"))
        .unwrap();
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive).unwrap();
    let mut children = Vec::new();
    for (index, seed) in seeds.iter().enumerate() {
        let output = directory.join(format!("result-{index}"));
        let _ = fs::remove_file(output.with_extension("ready"));
        children.push((
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "enrollment_process_worker"])
                .env("PLOYZ_TEST_ENROLLMENT_DIRECTORY", directory)
                .env("PLOYZ_TEST_ENROLLMENT_SEED", seed.to_string())
                .env("PLOYZ_TEST_ENROLLMENT_RESULT", &output)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
            output,
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while children
        .iter()
        .any(|(_, output)| !output.with_extension("ready").exists())
    {
        assert!(
            Instant::now() < deadline,
            "children did not reach the persistence boundary"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Children cannot complete while a separate process owns the stable lock.
    assert!(
        children
            .iter_mut()
            .all(|(child, _)| child.try_wait().unwrap().is_none())
    );
    drop(lock);
    children
        .into_iter()
        .map(|(mut child, output)| {
            assert!(child.wait().unwrap().success());
            serde_json::from_slice(&fs::read(output).unwrap()).unwrap()
        })
        .collect()
}

#[test]
fn independent_processes_serialize_stale_snapshots_and_resume_after_exit() {
    let identical = tempfile::tempdir().unwrap();
    let shared = concurrent(identical.path(), &[2, 2]);
    assert_eq!(shared.first().unwrap(), shared.get(1).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let assignments = concurrent(temp.path(), &[2, 3, 4]);
    let subnets: std::collections::HashSet<_> =
        assignments.iter().map(|a| a.machine.subnet).collect();
    assert_eq!(subnets.len(), 3);
    assert!(
        assignments
            .iter()
            .all(|a| a.machine.subnet != snapshot(1).machines.first().unwrap().subnet)
    );
    assert!(
        save_assignment(temp.path(), &request(5), &snapshot(1))
            .unwrap_err()
            .to_string()
            .contains("no free")
    );
    let resumed = concurrent(temp.path(), &[2, 2]);
    assert_eq!(resumed.first().unwrap(), assignments.first().unwrap());
    assert_eq!(resumed.first().unwrap(), resumed.get(1).unwrap());
    assert!(has_assignment(temp.path(), &snapshot(1), request(2).machine_id.unwrap()).unwrap());
}

#[test]
fn aliases_share_history_unrelated_scopes_do_not_and_inputs_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let original = snapshot(1);
    let saved = save_assignment(temp.path(), &request(2), &original).unwrap();
    // An alias using the newly enrolled peer (or a renamed context) retains scope.
    let mut alias = original.clone();
    alias.machines = vec![saved.machine.clone()];
    assert_eq!(
        save_assignment(temp.path(), &request(2), &alias).unwrap(),
        saved
    );
    let next = save_assignment(temp.path(), &request(3), &alias).unwrap();
    assert_ne!(next.machine.subnet, saved.machine.subnet);
    let unrelated = save_assignment(temp.path(), &request(4), &snapshot(9)).unwrap();
    assert_eq!(unrelated.machine.subnet, saved.machine.subnet);
    for field in 0..4 {
        let mut changed = request(2);
        match field {
            0 => changed.public_key = WireGuardPublicKey([99; 32]),
            1 => changed.storage = StorageChoice::Zfs,
            2 => changed.initial_policy.accepts_builds = false,
            _ => changed.advertised_endpoints.clear(),
        }
        assert!(save_assignment(temp.path(), &changed, &original).is_err());
    }
    assert_eq!(
        save_assignment(temp.path(), &request(2), &original).unwrap(),
        saved
    );
}
