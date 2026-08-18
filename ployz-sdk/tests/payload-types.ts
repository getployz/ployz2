import type {
  ConfigMount,
  ConfigSpec,
  ContainerObservation,
  DeviceMapping,
  DeviceReservation,
  HealthcheckSpec,
  RequestedServiceSpec,
  ResolvedServiceSpec,
  Ulimit,
  VolumeDriver,
} from "../generated/payloads";
import fixtures from "../generated/fixtures.json";

fixtures.volume_driver satisfies VolumeDriver;
fixtures.config_spec satisfies ConfigSpec;
fixtures.config_mount satisfies ConfigMount;
fixtures.device_mapping satisfies DeviceMapping;
fixtures.device_reservation satisfies DeviceReservation;
fixtures.ulimit satisfies Ulimit;
fixtures.requested_service_spec_typed.configs satisfies NonNullable<
  RequestedServiceSpec["configs"]
>;
fixtures.resolved_service_spec_typed.configs satisfies NonNullable<
  ResolvedServiceSpec["configs"]
>;
fixtures.requested_service_spec_typed.container
  .healthcheck satisfies HealthcheckSpec;
fixtures.container_observation_disabled_healthcheck
  .effective_healthcheck satisfies ContainerObservation["effective_healthcheck"];

// @ts-expect-error VolumeDriver options values are strings
const invalidDriver: VolumeDriver = { name: "nfs", options: { share: 1 } };
// @ts-expect-error DeviceMapping requires cgroup_permissions
const invalidDevice: DeviceMapping = {
  machine_path: "/dev/fuse",
  container_path: "/dev/fuse",
};
// @ts-expect-error ConfigSpec content is a byte array
const invalidConfig: ConfigSpec = { name: "settings", content: "port = 8080" };
// @ts-expect-error Ulimit requires hard
const invalidUlimit: Ulimit = { soft: 1024 };
// @ts-expect-error DeviceReservation count is a number
const invalidReservation: DeviceReservation = { count: "one" };
// @ts-expect-error HealthcheckSpec is tagged, not a string
const invalidHealthcheck: HealthcheckSpec = "disabled";
// @ts-expect-error configs is ConfigSpec[], not a number
const invalidConfigs: RequestedServiceSpec["configs"] = 1;
// @ts-expect-error effective_healthcheck is HealthcheckSpec | null, not a string
const invalidEffective: ContainerObservation["effective_healthcheck"] =
  "disabled";

void invalidDriver;
void invalidDevice;
void invalidConfig;
void invalidUlimit;
void invalidReservation;
void invalidHealthcheck;
void invalidConfigs;
void invalidEffective;
