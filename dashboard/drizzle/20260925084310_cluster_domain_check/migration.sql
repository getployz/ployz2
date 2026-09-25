ALTER TABLE "organization_cluster_domain" ADD COLUMN "traffic_issue" text;--> statement-breakpoint
ALTER TABLE "organization_cluster_domain" ADD COLUMN "checked_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "organization_cluster_domain" ADD CONSTRAINT "organization_cluster_domain_traffic_issue_check" CHECK ("traffic_issue" in ('no_servers', 'no_public_ip'));