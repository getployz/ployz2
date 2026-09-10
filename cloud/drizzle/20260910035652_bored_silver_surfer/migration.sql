-- Retired compatibility rows contain no connection authority.
DELETE FROM "organization_machine";--> statement-breakpoint
ALTER TABLE "organization_pairing" ADD COLUMN "founder_claim_machine_id" text;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD COLUMN "cluster_key" text NOT NULL;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD COLUMN "encrypted_tailcat" jsonb NOT NULL;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD CONSTRAINT "organization_machine_cluster_key_check" CHECK ("cluster_key" ~ '^[0-9a-f]{64}$');