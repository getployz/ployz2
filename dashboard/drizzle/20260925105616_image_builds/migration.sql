CREATE TABLE "environment_deployment_image_build" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid(),
	"organization_id" uuid NOT NULL,
	"deployment_id" uuid NOT NULL,
	"service_id" text NOT NULL,
	"image" text NOT NULL,
	"status" text DEFAULT 'building' NOT NULL,
	"machine_id" text,
	"encrypted_receipt" jsonb,
	"failure_message" text,
	"inngest_run_id" text NOT NULL,
	"finished_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "environment_deployment_image_build_service_unique" UNIQUE("deployment_id","service_id"),
	CONSTRAINT "environment_deployment_image_build_receipt_check" CHECK (("status" = 'built') = ("encrypted_receipt" is not null))
);
--> statement-breakpoint
ALTER TABLE "environment_deployment_build_step" ADD COLUMN "image" text DEFAULT '' NOT NULL;--> statement-breakpoint
ALTER TABLE "environment_deployment_secret" DROP COLUMN "encrypted_build_receipts";--> statement-breakpoint
ALTER TABLE "environment_deployment_build_step" DROP CONSTRAINT "environment_deployment_build_step_key_unique";--> statement-breakpoint
ALTER TABLE "environment_deployment_build_step" ADD CONSTRAINT "environment_deployment_build_step_key_unique" UNIQUE("deployment_id","image","build","key");--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD CONSTRAINT "environment_deployment_image_build_omhRa2gD2UgI_fkey" FOREIGN KEY ("organization_id") REFERENCES "organization"("id") ON DELETE CASCADE;--> statement-breakpoint
ALTER TABLE "environment_deployment_image_build" ADD CONSTRAINT "environment_deployment_image_build_Ss4XYtuD2fFu_fkey" FOREIGN KEY ("deployment_id") REFERENCES "environment_deployment"("id") ON DELETE CASCADE;--> statement-breakpoint
SELECT organization_change_attach('environment_deployment_image_build', 'organization_id', 'id');
