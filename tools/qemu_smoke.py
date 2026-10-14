#!/usr/bin/env python3
"""Boot an isolated snapshot and verify kernel regression/desktop serial markers."""
import argparse
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--image', type=pathlib.Path, required=True)
    parser.add_argument('--disk', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--regression', action='store_true')
    parser.add_argument('--cpus', type=int, default=2)
    parser.add_argument('--memory', default='256M')
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    log = args.output.resolve() / 'serial.log'
    screenshot = args.output.resolve() / 'desktop.ppm'
    # Never accept markers/screenshots from an earlier run before QEMU opens
    # and truncates its output files.
    log.write_text('')
    screenshot.unlink(missing_ok=True)
    command = ['qemu-system-x86_64',
        '-drive', f'format=raw,file={args.image.resolve()},index=0,media=disk,snapshot=on',
        '-drive', f'format=raw,file={args.disk.resolve()},index=1,media=disk,snapshot=on',
        '-serial', f'file:{log}', '-monitor', 'stdio', '-display', 'none',
        '-m', args.memory, '-smp', str(args.cpus), '-vga', 'std',
        '-netdev', 'user,id=net0,restrict=on', '-device', 'rtl8139,netdev=net0',
        '-no-reboot', '-no-shutdown']
    with (args.output / 'qemu.log').open('w') as output:
        proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=output, stderr=output, text=True)
        try:
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                serial = log.read_text(errors='replace') if log.exists() else ''
                if any(marker in serial for marker in ['KERNEL PANIC', '[PANIC]', '[FATAL]', '[USER-FAULT]']):
                    raise RuntimeError(f'Kernel fault; see {log}\n{serial[-5000:]}')
                booted = '[DESKTOP] Auto-booted' in serial and '[DESKTOP] First frame presented.' in serial
                tested = not args.regression or '[REGRESSION] ALL PASS' in serial
                if booted and tested:
                    time.sleep(1)
                    proc.stdin.write(f'screendump {screenshot}\n')
                    proc.stdin.flush()
                    for _ in range(20):
                        if screenshot.exists() and screenshot.stat().st_size > 100: break
                        time.sleep(.1)
                    proc.stdin.write('quit\n')
                    proc.stdin.flush()
                    proc.wait(timeout=5)
                    if not screenshot.exists():
                        raise RuntimeError('QEMU did not produce a desktop screenshot')
                    print(f'PASS: desktop boot, {args.cpus} CPUs, {args.memory}; serial log: {log}')
                    for line in serial.splitlines():
                        if '[REGRESSION]' in line: print(line)
                    print(f'Screenshot: {screenshot}')
                    return
                if proc.poll() is not None:
                    qemu_err = (args.output / 'qemu.log').read_text(errors='replace') if (args.output / 'qemu.log').exists() else ''
                    raise RuntimeError(f'QEMU exited ({proc.returncode}); log:\n{qemu_err}')
                time.sleep(.1)
            raise RuntimeError(f'Boot/regression timeout; see {log}\n{serial[-5000:]}')
        finally:
            if proc.poll() is None:
                proc.terminate()
                try: proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()


if __name__ == '__main__':
    main()
