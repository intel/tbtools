-- PostgreSQL database tables for tracing.
--
-- See following for installing the database itself
--   https://www.postgresql.org/docs/
--   https://ubuntu.com/server/docs/install-and-configure-postgresql
--
-- The tool output needs to be processed first to be suitable for
-- inserting into the database:
--   $ scripts/process-devices.py devices.csv
--   $ scripts/process-trace.py trace.csv
--
-- Following commands can be used to create the database, tables and
-- insert the data:
--   $ createdb tracedb
--   $ psql tracedb
--   => \i scripts/tables.sql
--   => \copy device from 'devices.processed.csv' WITH (FORMAT CSV, HEADER TRUE)
--   => \copy trace_entry from 'trace.processed.entries.csv' WITH (FORMAT CSV, HEADER TRUE)
--   => \copy trace_field from 'trace.processed.fields.csv' WITH (FORMAT CSV, HEADER TRUE)
--
-- Once you are done with the data you can drop the database:
--   $ dropdb tracedb
DROP TABLE IF EXISTS device;
DROP TABLE IF EXISTS trace_entry;
DROP TABLE IF EXISTS trace_field;

CREATE TABLE device (
	domain INTEGER,
	route BIGINT,
	adapter INTEGER,
	index INTEGER,
	vendor INTEGER,
	device INTEGER,
	vendor_name VARCHAR(256),
	device_name VARCHAR(256),
	type VARCHAR(32) NOT NULL,

	PRIMARY KEY (domain, route)
);

CREATE TABLE trace_entry (
	entry INTEGER,
	timestamp NUMERIC(12, 6),
	datetime TIMESTAMP,
	function VARCHAR(16),
	dropped BOOLEAN,
	pdf VARCHAR(32),
	cs VARCHAR(32),
	domain INTEGER,
	route BIGINT,
	adapter INTEGER,
	adapter_type VARCHAR(32),

	PRIMARY KEY(entry)
);

CREATE TABLE trace_field (
	entry INTEGER,
	field_offset INTEGER,
	data_offset INTEGER,
	value BIGINT,
	name VARCHAR(256),

	PRIMARY KEY (entry, field_offset)
);
