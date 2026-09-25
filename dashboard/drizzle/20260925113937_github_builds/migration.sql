CREATE TABLE "organization_build_order" (
	"organization_id" uuid PRIMARY KEY,
	"build_order" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "organization_build_order_check" CHECK ("build_order" in ('servers-only', 'github-then-servers', 'github-only'))
);
--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "builder" text DEFAULT 'server' NOT NULL;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "github_run_id" bigint;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "github_run_url" text;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "github_workflow_ref" text;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "checked_in_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "grant_id" text;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "fingerprint" text;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD COLUMN "platforms" text[];--> statement-breakpoint
ALTER TABLE "organization_build_order" ADD CONSTRAINT "organization_build_order_organization_id_organization_id_fkey" FOREIGN KEY ("organization_id") REFERENCES "organization"("id") ON DELETE CASCADE;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD CONSTRAINT "environment_deployment_image_build_builder_check" CHECK ("builder" in ('server', 'github'));--> statement-breakpoint
SELECT organization_change_attach('organization_build_order', 'organization_id', 'organization_id');