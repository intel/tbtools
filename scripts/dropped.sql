-- Find all packets that were dropped.
--
--   $ psql tracedb
--   => \i scripts/dropped.sql
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
    e.dropped = TRUE;
