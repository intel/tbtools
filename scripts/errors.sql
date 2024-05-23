-- Finds all error notifications from the trace data. These are all
-- notifications where event_code is not any of the following:
--
--   7 = HP_ACK
--   32 = DP_BW
--   33 = ROP_CMPLT
--   34 = POP_CMPLT
--   35 = PCIE_WAKE
--   36 = DP_CON_CHANGE
--   37 = DPTX_DISCOVERY
--   38 = LINK_RECOVERY
--   39 = ASYM_LINK
--
--   $ psql tracedb
--   => \i scripts/errors.sql
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
    e.function,
    e.pdf,
    to_hex(f.value) AS value,
    (f.value::bit(32) & x'000000ff'::bit(32))::integer AS event_code,
    ((f.value::bit(32) & x'00003f00'::bit(32)) >> 8)::integer AS event_info,
    ((f.value::bit(32) & x'c0000000'::bit(32)) >> 30)::integer AS PG
FROM
    trace_field f, trace_entry e, device d
WHERE
    f.entry = e.entry AND
    e.domain = d.domain AND
    e.route = d.route AND
    d.type = 'Router' AND
    e.pdf = 'Notification Packet' AND
    (f.value::bit(32) & x'000000ff'::bit(32))::integer NOT IN (7, 32, 33, 34, 35, 36, 37, 38, 39) AND
    f.field_offset = 2;
