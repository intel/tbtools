-- Find non-spec register access from trace output.
--
--   $ psql tracedb
--   => \i scripts/nonspec-register.sql
--
-- Check for access with no known register name. These are all non-spec
-- registers.
SELECT
    e.entry,
    e.timestamp,
    e.domain,
    to_hex(e.route) AS route,
    e.adapter,
    e.adapter_type,
    to_hex(d.vendor) AS vendor,
    to_hex(d.device) AS device ,
    d.vendor_name,
    d.device_name,
    e.pdf,
    e.cs,
    f.field_offset,
    to_hex(f.data_offset) AS data_offset,
    to_hex(f.value) AS value,
    f.name
FROM
    trace_field f, trace_entry e, device d
WHERE
    f.entry = e.entry AND
    e.domain = d.domain AND
    e.route = d.route AND
    d.type = 'Router' AND
    d.generation = 'USB4' AND
    f.entry IN (
        SELECT DISTINCT entry
        FROM trace_field
        WHERE data_offset IS NOT NULL AND name IS NULL
        ORDER BY entry
    );
