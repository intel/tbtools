#!/usr/bin/env python3

import argparse
import csv
import os
import sys
import util

def write_row(fields, writer, field_names):
    for field in fields:
        f = {key: field[key] for key in field_names if key in field}
        # Convert to integer
        util.convert_int(f, 'offset')
        util.convert_int(f, 'data_offset')
        util.convert_int(f, 'value')
        # Write them
        writer.writerow(f)

def process_trace(args):
    base = os.path.basename(args.trace)
    base = os.path.splitext(base)[0]
    entries = base + '.processed.entries.csv'
    fields = base + '.processed.fields.csv'

    with open(args.trace) as input_file, \
         open(entries, 'w') as entries_file, \
         open(fields, 'w') as fields_file:
        reader = csv.DictReader(input_file)
        entry_names = [
            'entry',
            'timestamp',
            'datetime',
            'function',
            'dropped',
            'pdf',
            'cs',
            'domain',
            'route',
            'adapter',
            'adapter_type',
        ]
        entries_writer = csv.DictWriter(entries_file, fieldnames=entry_names)
        entries_writer.writeheader()
        field_names = [
            'entry',
            'offset',
            'data_offset',
            'value',
            'name',
        ]
        fields_writer = csv.DictWriter(fields_file, fieldnames=field_names)
        fields_writer.writeheader()
        line = None
        for row in reader:
            if not line:
                # First line in the file
                line = row['entry']
                fields = [row]
            else:
                if line != row['entry']:
                    # Write the entries first
                    e = {key: row[key] for key in entry_names if key in row}
                    util.convert_timestamp(e, 'datetime')
                    util.convert_int(e, 'route')
                    entries_writer.writerow(e)
                    # Then the fields
                    write_row(fields, fields_writer, field_names)
                    line = row['entry']
                    fields = [row]
                else:
                    fields.append(row)

        # Last one
        write_row(fields, fields_writer, field_names)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(prog='process-trace',
                                     description='Split the trace CSV output into separate files.',
                                     epilog="""
Splits trace output into separate files, more suitable for inserting
into database. From file TRACE generates two files
TRACE.processed.entries.csv and TRACE.processed.fields.csv. These have
matching first field that is the trace line number.
                                     """)
    parser.add_argument('trace');
    args = parser.parse_args()

    process_trace(args)
