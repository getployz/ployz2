-- Coordinated fresh cutover only: stop old writers and retain existing claims unchanged.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM "organization_pairing") THEN
    RAISE EXCEPTION 'Tailcat enrollment cutover requires no existing Organization pairings.'
      USING ERRCODE = '55000';
  END IF;
END $$;--> statement-breakpoint
-- Retired compatibility rows contain no connection authority.
DELETE FROM "organization_machine";--> statement-breakpoint
ALTER TABLE "organization_pairing" ADD COLUMN "founder_claim_machine_id" text NOT NULL;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD COLUMN "cluster_key" text NOT NULL;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD COLUMN "encrypted_tailcat" jsonb NOT NULL;--> statement-breakpoint
ALTER TABLE "organization_machine" ADD CONSTRAINT "organization_machine_cluster_key_check" CHECK ("cluster_key" ~ '^[0-9a-f]{64}$');
--> statement-breakpoint
ALTER TABLE "organization_pairing" ADD CONSTRAINT "organization_pairing_founder_claim_machine_id_check" CHECK ("founder_claim_machine_id" ~ '^[0-9a-f]{32}$');
