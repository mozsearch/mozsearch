#!/usr/bin/env python3

# Log the instance's resource usage to stderr (the indexer log, since main.sh
# starts this in the background), so that we can tell how much headroom an
# instance type had: memory (as "used", which is MemTotal - MemAvailable, and
# the anonymous memory in it, which can't just be dropped like the page
# cache), swap, CPU, load, and disk usage, and the processes using the most
# memory.  It samples every 30 seconds and logs a line every 5 minutes and
# whenever the memory used reaches a new peak (by at least 1 GiB, at most once
# a minute).  On SIGUSR1 (see upload-log.sh) or SIGTERM, it logs the peaks so
# far, with the processes using the most memory at the peak.

import os
import signal
import sys
import time
from datetime import datetime, timezone

SAMPLE_SECONDS = 30
LOG_SECONDS = 300
PEAK_STEP_KIB = 1024 * 1024
PEAK_LOG_SECONDS = 60
GIB = 1024 * 1024


def now():
    return datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


def log(message):
    print(f'[{now()} resources] {message}', file=sys.stderr, flush=True)


def meminfo():
    info = {}
    with open('/proc/meminfo') as f:
        for line in f:
            key, value = line.split(':', 1)
            info[key] = int(value.split()[0])
    return info


def cpu_times():
    with open('/proc/stat') as f:
        fields = [int(x) for x in f.readline().split()[1:]]
    idle = fields[3] + fields[4]
    return sum(fields), idle, fields[4]


def disk(path):
    try:
        st = os.statvfs(path)
    except OSError:
        return None
    total = st.f_blocks * st.f_frsize
    return total - st.f_bavail * st.f_frsize, total


def processes(count=5):
    procs = []
    for pid in os.listdir('/proc'):
        if not pid.isdigit():
            continue
        try:
            with open(f'/proc/{pid}/status') as f:
                status = dict(line.split(':', 1) for line in f if ':' in line)
            with open(f'/proc/{pid}/cmdline', 'rb') as f:
                cmdline = f.read().replace(b'\0', b' ').decode(errors='replace').strip()
        except OSError:
            continue
        kib = lambda key: int(status.get(key, '0 kB').split()[0])
        anon, file, swap = kib('RssAnon'), kib('RssFile'), kib('VmSwap')
        if anon + swap == 0:
            continue
        name = os.path.basename(cmdline.split(' ', 1)[0]) if cmdline else status['Name'].strip()
        procs.append((anon + swap, name, anon, file, swap))
    procs.sort(reverse=True)
    return procs[:count]


def format_procs(procs):
    return ', '.join(
        f'{name} anon {anon / GIB:.1f} file {file / GIB:.1f} swap {swap / GIB:.1f}'
        for _, name, anon, file, swap in procs
    )


def gib(kib):
    return f'{kib / GIB:.1f}'


class Monitor:
    def __init__(self):
        self.peaks = {}
        self.last_cpu = cpu_times()
        self.last_log = 0
        self.last_peak_log = 0
        self.logged_peak_used = 0

    def peak(self, key, value, when, extra=None, lowest=False):
        old = self.peaks.get(key)
        if old is None or (value < old[0] if lowest else value > old[0]):
            self.peaks[key] = (value, when, extra)

    def sample(self):
        when = now()
        mem = meminfo()
        used = mem['MemTotal'] - mem['MemAvailable']
        anon = mem.get('AnonPages', 0)
        swap = mem['SwapTotal'] - mem['SwapFree']
        total, idle, iowait = cpu_times()
        last_total, last_idle, last_iowait = self.last_cpu
        self.last_cpu = (total, idle, iowait)
        elapsed = max(total - last_total, 1)
        busy = 100 * (elapsed - (idle - last_idle)) / elapsed
        io = 100 * (iowait - last_iowait) / elapsed
        with open('/proc/loadavg') as f:
            load = float(f.read().split()[0])
        disks = {path: disk(path) for path in ['/', '/index']}
        procs = processes()

        self.peak('used', used, when, procs)
        self.peak('anon', anon, when)
        self.peak('swap', swap, when)
        self.peak('busy', busy, when)
        self.peak('load', load, when)
        for path, usage in disks.items():
            if usage:
                self.peak(f'{path} used', usage[0], when)

        line = (
            f'mem used {gib(used)} GiB of {gib(mem["MemTotal"])} (anon {gib(anon)}), '
            f'swap {gib(swap)} of {gib(mem["SwapTotal"])}, '
            f'cpu {busy:.0f}% busy {io:.0f}% iowait, load {load:.2f}, '
            + ', '.join(
                f'{path} {usage[0] / 2**30:.0f}/{usage[1] / 2**30:.0f} GiB'
                for path, usage in disks.items()
                if usage
            )
            + f'; top: {format_procs(procs)}'
        )
        t = time.monotonic()
        new_peak = used >= self.logged_peak_used + PEAK_STEP_KIB
        if new_peak and t - self.last_peak_log >= PEAK_LOG_SECONDS:
            self.logged_peak_used = used
            self.last_peak_log = t
            self.last_log = t
            log(f'new peak: {line}')
        elif t - self.last_log >= LOG_SECONDS:
            self.last_log = t
            log(line)

    def summary(self, *_):
        parts = []
        for key in ['used', 'anon', 'swap']:
            if key in self.peaks:
                value, when, _ = self.peaks[key]
                parts.append(f'mem {key} {gib(value)} GiB at {when}' if key != 'swap'
                             else f'swap {gib(value)} GiB at {when}')
        for key, fmt in [('busy', '{:.0f}%'), ('load', '{:.2f}')]:
            if key in self.peaks:
                value, when, _ = self.peaks[key]
                parts.append(f'{key} {fmt.format(value)} at {when}')
        for path in ['/', '/index']:
            key = f'{path} used'
            if key in self.peaks:
                value, when, _ = self.peaks[key]
                parts.append(f'{path} {value / 2**30:.0f} GiB at {when}')
        log('peaks: ' + '; '.join(parts))
        if 'used' in self.peaks:
            log('top at peak mem used: ' + format_procs(self.peaks['used'][2]))


def main():
    monitor = Monitor()
    signal.signal(signal.SIGUSR1, monitor.summary)

    def terminate(*args):
        monitor.summary()
        sys.exit(0)

    signal.signal(signal.SIGTERM, terminate)
    while True:
        try:
            monitor.sample()
        except Exception as e:
            log(f'sampling failed: {e}')
        time.sleep(SAMPLE_SECONDS)


if __name__ == '__main__':
    main()
