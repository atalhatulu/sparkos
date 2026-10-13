#!/usr/bin/env python3
"""External PS/2 input trial; uses disposable disks and saves visual evidence.
Screenshots are observations, not assertions that each application action worked.
"""
import argparse, json, pathlib, socket, subprocess, time
from PIL import Image

p = argparse.ArgumentParser()
p.add_argument('--image', required=True, type=pathlib.Path)
p.add_argument('--disk', required=True, type=pathlib.Path)
p.add_argument('--output', required=True, type=pathlib.Path)
p.add_argument('--accel', choices=['kvm', 'tcg'], default='tcg')
a = p.parse_args(); out = a.output.resolve(); out.mkdir(parents=True, exist_ok=True)
serial = out/'serial.log'; serial.write_text('')
qmp_path = out/'qmp.sock'; qmp_path.unlink(missing_ok=True)
events = []
proc = subprocess.Popen(['qemu-system-x86_64', '-accel', a.accel,
    '-drive', f'format=raw,file={a.image.resolve()},index=0,snapshot=on',
    '-drive', f'format=raw,file={a.disk.resolve()},index=1,snapshot=on',
    '-m','256M','-smp','2','-vga','std','-display','none',
    '-qmp',f'unix:{qmp_path},server=on,wait=off','-serial',f'file:{serial}',
    '-netdev','user,id=n0,restrict=on','-device','rtl8139,netdev=n0','-no-reboot','-no-shutdown'],
    stdout=(out/'qemu.log').open('w'), stderr=subprocess.STDOUT)
start=time.monotonic()
try:
    while not qmp_path.exists():
        if proc.poll() is not None: raise RuntimeError((out/'qemu.log').read_text())
        if time.monotonic()-start>10: raise RuntimeError('QMP timeout')
        time.sleep(.05)
    sock=socket.socket(socket.AF_UNIX); sock.connect(str(qmp_path)); stream=sock.makefile('rwb',buffering=0)
    stream.readline()
    def qmp(name,args=None):
        stream.write((json.dumps({'execute':name,'arguments':args or {}})+'\n').encode())
        while True:
            msg=json.loads(stream.readline())
            if 'error' in msg: raise RuntimeError(msg)
            if 'return' in msg: return msg['return']
    qmp('qmp_capabilities')
    def hmp(cmd): return qmp('human-monitor-command',{'command-line':cmd})
    def key(k): hmp('sendkey '+k); time.sleep(.18)
    def shot(name):
        path=out/(name+'.ppm'); qmp('screendump',{'filename':str(path)})
        Image.open(path).save(out/(name+'.png'))
        events.append({'step':name,'time_s':round(time.monotonic()-start,3)})
        print(name,flush=True)
    while '[DESKTOP] First frame presented.' not in serial.read_text(errors='replace'):
        if time.monotonic()-start>60: raise RuntimeError('desktop timeout')
        time.sleep(.05)
    shot('01-desktop')
    # Initial guest mouse is (320,180); driver doubles PS/2 deltas.
    hmp('mouse_move -60 -47'); time.sleep(.15)
    hmp('mouse_button 1'); time.sleep(.12)
    for i in range(40):
        hmp('mouse_move 5 2'); time.sleep(.025)
        if i % 5 == 4: shot(f'02-drag-{i+1:02}')
    hmp('mouse_button 0'); time.sleep(.3); shot('03-drag-released')
    # Resize using the bottom-right border, then return to the titlebar.
    for _ in range(2): hmp('mouse_move 79 71'); time.sleep(.08)
    hmp('mouse_button 1'); time.sleep(.12)
    for _ in range(10): hmp('mouse_move 5 3'); time.sleep(.035)
    hmp('mouse_button 0'); time.sleep(.3); shot('03b-resized')
    for _ in range(2): hmp('mouse_move -10 -86'); time.sleep(.08)
    hmp('mouse_button 1'); time.sleep(.08); hmp('mouse_button 0'); time.sleep(.3)
    shot('03c-minimized')
    key('alt-tab'); time.sleep(.2); shot('03d-first-restore-attempt')

    for _ in range(2): hmp('mouse_move -94 0'); time.sleep(.08)
    # Files is now at approximately (480,236); double-click titlebar.
    hmp('mouse_button 1'); time.sleep(.08); hmp('mouse_button 0'); time.sleep(.08)
    hmp('mouse_button 1'); time.sleep(.08); hmp('mouse_button 0'); time.sleep(.3)
    shot('04-maximize')
    key('alt-tab'); shot('05-switch-terminal')
    for k in ['h','e','l','p','ret']: key(k)
    shot('06-terminal-help')
    # Launcher starts on Terminal. Open Editor through real keyboard events.
    key('ctrl-esc'); key('down'); key('down'); key('ret'); time.sleep(.3)
    for k in ['t','e','s','t','spc','1','2','3']: key(k)
    shot('07-editor-typed')
    key('alt-f4'); shot('08-unsaved-dialog')
    key('alt-f4'); shot('09-repeat-close')
    key('esc'); shot('10-cancel-close')
    key('alt-f4'); key('d'); shot('11-discard-close')
    # Launcher retains Editor selection: next entry is Task Manager.
    key('ctrl-esc'); key('down'); key('ret'); time.sleep(.3)
    shot('12-task-manager')
    key('down'); key('f5'); shot('13-task-manager-refresh')
    key('alt-f4'); shot('14-task-manager-closed')
    # Next entry is Settings.
    key('ctrl-esc'); key('down'); key('ret'); time.sleep(.3)
    key('spc'); shot('15-settings-light')
    key('spc'); key('up'); shot('16-settings-system')
    key('alt-f4'); shot('17-settings-closed')
    # Repeated launch/close through keyboard, looking for UI stalls and leaks.
    for i in range(10):
        key('f1'); time.sleep(.08); key('alt-f4')
    shot('18-after-ten-terminal-cycles')
    time.sleep(3); key('ctrl-esc'); shot('19-final-launcher-response')
    faults=[line for line in serial.read_text(errors='replace').splitlines()
            if any(x in line for x in ['[PANIC]','[USER-FAULT]','[FATAL]','KERNEL PANIC'])]
    (out/'trial.json').write_text(json.dumps({'accel':a.accel,'events':events,'faults':faults},indent=2))
    print('Trial completed; inspect screenshots and logs. Fault markers:',faults,flush=True)
    qmp('quit'); proc.wait(timeout=5)
finally:
    if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
