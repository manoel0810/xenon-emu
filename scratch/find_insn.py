import urllib.request, re

url = 'https://raw.githubusercontent.com/xenia-project/xenia/master/src/xenia/cpu/ppc/ppc_opcode_disasm_gen.cc'
d = urllib.request.urlopen(url).read().decode('utf-8')

for line in d.splitlines():
    m = re.search(r'INSTRUCTION\s*\(\s*(0x[0-9a-fA-F]+)\s*,\s*"([^"]+)"\s*,\s*(\w+)', line)
    if m:
        base_hex = int(m.group(1), 16)
        name = m.group(2)
        if name in ['stvehx', 'mfmsr']:
            xo = (base_hex >> 1) & 0x3FF
            print(f"{name:<15} base=0x{base_hex:08X} xo={xo}")
