-- Check for writes to lane 1 adapter basic config spaces. This is
-- forbidden in the spec. This only applies Intel routers for now. We
-- need to expose adapters as well to figure out the lane 1 adapters of
-- other vendors (or hard-code them).
--
--   $ psql tracedb
--   => \i scripts/lane-1.sql
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
    e.pdf = 'Write Request' AND
    e.cs = 'Adapter' AND
    e.adapter_type = 'Lane' AND
    -- Intel lane 1 adapters
    e.adapter IN (2, 4, 6, 8) AND
    f.name LIKE 'ADP_CS_%';
