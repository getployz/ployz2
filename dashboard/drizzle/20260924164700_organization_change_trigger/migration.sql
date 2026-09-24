-- One log row per statement per Organization. Arguments: the Organization column, then the key columns
-- (joined with ':' into the collection key). Over 100 keys set all_rows and drop the ids.
CREATE FUNCTION organization_change_log() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  key_expression text;
  old_keys text := 'SELECT NULL::uuid AS organization_id, NULL::text AS key WHERE false';
  new_keys text := old_keys;
BEGIN
  SELECT string_agg(format('%I::text', key_column), ' || '':'' || ')
    INTO key_expression FROM unnest(TG_ARGV[1:TG_NARGS - 1]) AS key_column;
  IF TG_OP IN ('INSERT', 'UPDATE') THEN
    new_keys := format('SELECT %I AS organization_id, %s AS key FROM new_rows', TG_ARGV[0], key_expression);
  END IF;
  IF TG_OP IN ('UPDATE', 'DELETE') THEN
    old_keys := format('SELECT %I AS organization_id, %s AS key FROM old_rows', TG_ARGV[0], key_expression);
  END IF;
  EXECUTE format($sql$
    INSERT INTO organization_change (organization_id, source_table, changed_ids, deleted_ids, all_rows)
    SELECT organization_id, %L,
      CASE WHEN count(*) > 100 THEN '{}' ELSE coalesce(array_agg(key) FILTER (WHERE NOT deleted), '{}') END,
      CASE WHEN count(*) > 100 THEN '{}' ELSE coalesce(array_agg(key) FILTER (WHERE deleted), '{}') END,
      count(*) > 100
    FROM (
      SELECT DISTINCT organization_id, key, false AS deleted FROM (%s) changed
      UNION ALL
      (SELECT organization_id, key, true FROM (%s) gone EXCEPT SELECT organization_id, key, true FROM (%s) changed)
    ) keys
    GROUP BY organization_id
  $sql$, TG_TABLE_NAME, new_keys, old_keys, new_keys);
  RETURN NULL;
END
$$;--> statement-breakpoint
-- Transition tables allow one event per trigger, so each table gets three.
CREATE FUNCTION organization_change_attach(target regclass, organization_column text, VARIADIC key_columns text[])
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  arguments text := (SELECT string_agg(quote_literal(argument), ', ') FROM unnest(organization_column || key_columns) AS argument);
BEGIN
  EXECUTE format('CREATE TRIGGER organization_change_insert AFTER INSERT ON %s REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION organization_change_log(%s)', target, arguments);
  EXECUTE format('CREATE TRIGGER organization_change_update AFTER UPDATE ON %s REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION organization_change_log(%s)', target, arguments);
  EXECUTE format('CREATE TRIGGER organization_change_delete AFTER DELETE ON %s REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION organization_change_log(%s)', target, arguments);
END
$$;--> statement-breakpoint
SELECT organization_change_attach('service', 'organization_id', 'id');
