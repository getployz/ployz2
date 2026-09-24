CREATE TABLE "organization_change" (
	"seq" bigint PRIMARY KEY GENERATED ALWAYS AS IDENTITY (sequence name "organization_change_seq_seq" INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1),
	"xid" xid8 DEFAULT pg_current_xact_id() NOT NULL,
	"organization_id" uuid NOT NULL,
	"source_table" text NOT NULL,
	"changed_ids" text[] NOT NULL,
	"deleted_ids" text[] NOT NULL,
	"all_rows" boolean NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE INDEX "organization_change_organization_id_xid_idx" ON "organization_change" ("organization_id","xid");