with open('SSDB_ACRPRDCT.aml', 'rb') as f:
    d = f.read()

chunk = d[30250:34000]
import re
for m in re.finditer(rb'[A-Za-z0-9_]{3,}', chunk):
    print(f"+0x{m.start():X}: {m.group().decode('latin1')}")
