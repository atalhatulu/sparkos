#!/usr/bin/env python3
"""Serial, repeatable PS/2 workloads against the opt-in perf-metrics image.
Internal timings use guest calibrated TSC. Host elapsed is not input latency.
"""
import argparse
import json
import os
import pathlib
import re
import socket
import subprocess
import time
from PIL import Image


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--image', type=pathlib.Path, required=True)
    ap.add_argument('--disk', type=pathlib.Path, required=True)
    ap.add_argument('--output', type=pathlib.Path, required=True)
    ap.add_argument('--accel', choices=['kvm', 'tcg'], default='kvm')
    ap.add_argument('--idle-seconds', type=float, default=60)
    ap.add_argument('--record', choices=['on', 'off'], default='on')
    args = ap.parse_args()
    out = args.output.resolve(); out.mkdir(parents=True, exist_ok=True)
    serial = out / 'serial.log'; serial.write_text('')
    sockpath = out / 'qmp.sock'; sockpath.unlink(missing_ok=True)
    results = []
    command = ['qemu-system-x86_64','-accel',args.accel,
        '-drive',f'format=raw,file={args.image.resolve()},index=0,snapshot=on',
        '-drive',f'format=raw,file={args.disk.resolve()},index=1,snapshot=on',
        '-m','256M','-smp','2','-vga','std','-display','none',
        '-qmp',f'unix:{sockpath},server=on,wait=off','-serial',f'file:{serial}',
        '-netdev','user,id=n0,restrict=on','-device','rtl8139,netdev=n0','-no-reboot','-no-shutdown']
    with (out/'qemu.log').open('w') as output:
        proc = subprocess.Popen(command, stdout=output, stderr=output)
        try:
            def wait_for(predicate, timeout=60):
                deadline = time.monotonic() + timeout
                while not predicate():
                    if proc.poll() is not None: raise RuntimeError('QEMU exited')
                    if time.monotonic() > deadline: raise RuntimeError('Timeout; see serial.log')
                    time.sleep(.01)
            wait_for(sockpath.exists, 10)
            sock = socket.socket(socket.AF_UNIX); sock.settimeout(10); sock.connect(str(sockpath))
            stream = sock.makefile('rwb', buffering=0); stream.readline()
            def qmp(name, arguments=None):
                stream.write((json.dumps({'execute':name,'arguments':arguments or {}})+'\n').encode())
                while True:
                    reply = json.loads(stream.readline())
                    if 'error' in reply: raise RuntimeError(reply)
                    if 'return' in reply: return reply['return']
            qmp('qmp_capabilities')
            def hmp(cmd):
                reply = qmp('human-monitor-command', {'command-line':cmd})
                if reply: raise RuntimeError(reply)
            def key(name, delay=.09): hmp('sendkey '+name+' 30'); time.sleep(delay)
            def move(dx, dy, delay=.025): hmp(f'mouse_move {dx} {dy}'); time.sleep(delay)
            def button(down): hmp('mouse_button '+('1' if down else '0')); time.sleep(.08)
            def shot(name):
                p = out/(name+'.ppm'); qmp('screendump', {'filename':str(p)})
                Image.open(p).save(out/(name+'.png')); p.unlink()
            def cpu_seconds():
                fields = pathlib.Path(f'/proc/{proc.pid}/stat').read_text().rsplit(')', 1)[1].split()
                return (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')
            def workload(name, fn):
                before = serial.read_text(errors='replace')
                if args.record == 'on':
                    key('f9'); wait_for(lambda: serial.read_text().count('[PERF] BEGIN') > before.count('[PERF] BEGIN'))
                cpu_start = cpu_seconds()
                started = time.monotonic(); fn(); time.sleep(.15)
                host_elapsed = time.monotonic() - started
                host_cpu = cpu_seconds() - cpu_start
                if args.record == 'on':
                    key('f10'); wait_for(lambda: serial.read_text().count('[PERF] END') > before.count('[PERF] END'))
                delta = serial.read_text(errors='replace')[len(before):]
                metrics = {}
                for line in delta.splitlines():
                    if line.startswith('[PERF] metric='):
                        parts = dict(re.findall(r'(\w+)=([\w.-]+)',line))
                        label = parts.pop('metric'); metrics[label] = {k:int(v) for k,v in parts.items()}
                reports = [line for line in delta.splitlines() if line.startswith('[PERF] REPORT')]
                summary = {k:int(v) for k,v in re.findall(r'(\w+)=(\d+)',reports[-1])} if reports else {}
                if args.record == 'on' and len(metrics) != 13: raise RuntimeError('Incomplete metrics report')
                shot(name)
                result = dict(scenario=name, host_elapsed_s=host_elapsed, qemu_cpu_s=host_cpu, summary=summary, metrics=metrics)
                results.append(result)
                (out/'results.json').write_text(json.dumps(dict(accel=args.accel, record=args.record, clock=[line for line in serial.read_text().splitlines() if '[PERF] clock=' in line or '[PERF] WARNING' in line], scenarios=results),indent=2))
                print(name, json.dumps(summary), flush=True)
            wait_for(lambda: '[DESKTOP] First frame presented.' in serial.read_text(errors='replace'))
            time.sleep(.4)
            workload('idle', lambda: time.sleep(args.idle_seconds))
            move(-60,-47); button(True) # Files titlebar at (200,86)
            def drag():
                for i in range(120): move(2 if (i//30)%2 == 0 else -2, 0)
            workload('drag', drag); button(False)
            # Return point is unchanged; reach Files bottom-right resize corner.
            for _ in range(2): move(79,71,.08)
            button(True)
            def resize():
                for i in range(120):
                    sign = 1 if (i//30)%2 == 0 else -1
                    move(2*sign,sign)
                button(False)
            workload('resize', resize)
            key('alt-tab') # Focus initial Terminal.
            def typing():
                for _ in range(8):
                    for k in ['h','e','l','p','ret']: key(k,.06)
            workload('terminal-typing', typing)
            def cycles():
                for _ in range(20): key('f1',.15); key('alt-f4',.15)
            workload('terminal-open-close',cycles)
            # End with one more real response check.
            key('ctrl-esc'); shot('final-launcher')
            faults = [s for s in serial.read_text(errors='replace').splitlines()
                if any(x in s for x in ['[PANIC]','[USER-FAULT]','[FATAL]','KERNEL PANIC'])]
            if faults: raise RuntimeError(faults)
            qmp('quit'); proc.wait(timeout=5)
        finally:
            if proc.poll() is None:
                proc.terminate()
                try: proc.wait(timeout=5)
                except subprocess.TimeoutExpired: proc.kill(); proc.wait()


if __name__ == '__main__': main()
