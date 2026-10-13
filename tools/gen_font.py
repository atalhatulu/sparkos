import os
import re

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
FONT_H = os.path.join(SCRIPT_DIR, "font.h")
TARGET_RS = os.path.join(SCRIPT_DIR, "font_ascii.rs")

with open(FONT_H, "r") as f:
    content = f.read()

# Extract the array content
matches = re.findall(r'\{([0-9A-Fa-fx,\s]+)\}', content)

rust_array = "pub static FONT: [[u8; 8]; 128] = [\n"
for i in range(128):
    if i < len(matches):
        vals = [v.strip() for v in matches[i].split(',') if v.strip()]
        if len(vals) == 8:
            rust_array += f"    [{', '.join(vals)}],\n"
        else:
            rust_array += "    [0; 8],\n"
    else:
        rust_array += "    [0; 8],\n"

rust_array += "];\n"

with open(TARGET_RS, "w") as f:
    f.write(rust_array)

