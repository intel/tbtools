#!/usr/bin/env python3

import heapq
import os
import re
import sys

def merge_logs(*logs):
    files = [open(file, 'r') for file in logs]
    heap = []
    count = 0

    pattern = re.compile(r"^\[\s*(\d+\.\d+)\]")

    for file in files:
        entry = ()
        while True:
            line = file.readline()
            if not line:
                # Add the last one if multiline
                if entry:
                    heapq.heappush(heap, entry)
                file.close()
                break;
            line = line.rstrip('\n')
            match = pattern.match(line)
            if match:
                if entry:
                    heapq.heappush(heap, entry)
                # Make it sorted by timestamp but also insertion order
                # so that's why we use count here. See more from heapq
                # documentation (specifically the Priority Queue section).
                entry = (float(match.group(1)), count, line)
                count += 1
            else:
                entry = (entry[0], entry[1], entry[2] + "\n" + line)

    while heap:
        print(heapq.heappop(heap)[2])

if __name__ == "__main__":
    logs = sys.argv[1:]
    if not logs:
        print("Usage: " + os.path.basename(__file__) + " LOG1 [LOG2...]")
        print()
        print("Merges two or more log files into one")
        sys.exit(1)

    merge_logs(*logs)
