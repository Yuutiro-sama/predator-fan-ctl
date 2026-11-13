for fname in ['dsdt.aml', 'SSDB_ACRPRDCT.aml']:
    with open(fname, 'rb') as f:
        d = f.read()
    for name in [b'CPUF', b'GPUF', b'CTMP', b'GTMP', b'NWFS', b'GGSI']:
        pos = [m.start() for m in re.finditer(name, d)]
        if pos:
            print(f"{fname}: {name.decode()} found at {pos}")
