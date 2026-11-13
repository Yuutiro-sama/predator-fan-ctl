with open('SSDB_ACRPRDCT.aml', 'rb') as f:
    d = f.read()

import re
matches = re.findall(rb'EC0_[A-Za-z0-9_]+', d)
print("EC0 methods:", sorted(set(m.decode("latin1") for m in matches)))
