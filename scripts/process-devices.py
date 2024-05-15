#!/usr/bin/env python3

import argparse
import csv
import os
import util

def process_devices(args):
    base = os.path.basename(args.devices)
    base = os.path.splitext(base)[0]
    devices = base + '.processed.csv'

    with open(args.devices) as input_file, open(devices, 'w') as output_file:
        reader = csv.DictReader(input_file)
        field_names = [
            'domain',
            'route',
            'adapter',
            'index',
            'vendor',
            'device',
            'vendor_name',
            'device_name',
            'type'
        ]
        devices_writer = csv.DictWriter(output_file, fieldnames=field_names)
        devices_writer.writeheader()
        for row in reader:
            util.convert_int(row, 'route')
            util.convert_int(row, 'vendor')
            util.convert_int(row, 'device')
            devices_writer.writerow(row)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(prog='process-devices',
                                     description='Process tblist -S output suitable for database.',
                                     epilog="""
Processes the tblist -S output into a format that is suitable for
inserting into a database. From file DEVICES generates a file called
DEVICES.processed.csv.
                                    """)
    parser.add_argument('devices');
    args = parser.parse_args()

    process_devices(args)
