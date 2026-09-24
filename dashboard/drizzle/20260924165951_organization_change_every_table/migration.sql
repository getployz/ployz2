-- Every organization-owned table logs its changes; `organization` is keyed by its own id.
-- Key columns match changeSources in src/collections/change-sources.ts.
SELECT organization_change_attach('organization', 'id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('project', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('user_project_preference', 'organization_id', 'project_id');
--> statement-breakpoint
SELECT organization_change_attach('resource_lineage', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_resource', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_canvas_node_position', 'organization_id', 'resource_type', 'resource_id');
--> statement-breakpoint
SELECT organization_change_attach('environment_deployment', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_deployment_event', 'organization_id', 'deployment_id');
--> statement-breakpoint
SELECT organization_change_attach('environment_saved_state_snapshot', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_node_config_snapshot', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_node_introduction', 'organization_id', 'node_type', 'node_id');
--> statement-breakpoint
SELECT organization_change_attach('volume_remove_attempt', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('organization_pairing', 'organization_id', 'organization_id');
--> statement-breakpoint
SELECT organization_change_attach('core_operation_event', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('core_operation_watch', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('enrollment_allocation', 'organization_id', 'cluster_key');
--> statement-breakpoint
SELECT organization_change_attach('environment_deployment_build_output', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_deployment_build_step', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('environment_deployment_secret', 'organization_id', 'environment_deployment_id');
--> statement-breakpoint
SELECT organization_change_attach('environment_node_config_snapshot_secret', 'organization_id', 'snapshot_id');
--> statement-breakpoint
SELECT organization_change_attach('environment_node_introduction_secret', 'organization_id', 'environment_id', 'node_type', 'node_id');
--> statement-breakpoint
SELECT organization_change_attach('github_environment_trigger', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('invitation', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('machine_enrollment_token', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('machine_remove_attempt', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('member', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('organization_billing_state', 'organization_id', 'organization_id');
--> statement-breakpoint
SELECT organization_change_attach('organization_machine', 'organization_id', 'machine_id');
--> statement-breakpoint
SELECT organization_change_attach('service_lineage', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('service_registry_credential', 'organization_id', 'service_id');
--> statement-breakpoint
SELECT organization_change_attach('teardown_attempt', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('variable', 'organization_id', 'id');
--> statement-breakpoint
SELECT organization_change_attach('variable_secret', 'organization_id', 'variable_id');
