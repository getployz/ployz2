ALTER TABLE "organization_pairing" ADD COLUMN "removal_started_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "organization_pairing" ADD COLUMN "removal_endpoints" jsonb;--> statement-breakpoint
ALTER TABLE "organization_pairing" ADD CONSTRAINT "organization_pairing_removal_shape_check" CHECK (
      ("removal_started_at" is null and "removal_endpoints" is null)
      or ("removal_started_at" is not null and "removal_endpoints" is not null and jsonb_typeof("removal_endpoints") = 'array')
    );
