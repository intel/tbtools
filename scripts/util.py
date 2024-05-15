import datetime

# Convert hex string into integer
def convert_int(fields, name):
    tmp = fields[name]
    if tmp:
        fields[name] = int(tmp, 16)

# Convert timestamp into datetime
def convert_timestamp(fields, name):
    tmp = fields[name]
    if tmp:
        dt = datetime.datetime.fromtimestamp(float(tmp))
        fields[name] = dt.strftime("%Y-%m-%dT%H:%M:%S")
