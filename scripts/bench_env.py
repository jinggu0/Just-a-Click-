"""Shared benchmark environment checks. Reads power state; never changes system settings."""
from contextlib import contextmanager
import csv
import ctypes
from datetime import datetime, timezone
import io
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ES_CONTINUOUS, ES_SYSTEM_REQUIRED = 0x80000000, 0x00000001
OVERLAY_KEY = r'SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes'
POWER_MODES = {'961cc777-2547-4f9d-8174-7d86181b8a7a': 'best_power_efficiency',
               'ded574b5-45a0-4f42-8737-46345c09c238': 'best_performance'}


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds')


def running_processes(tasklist_csv, blocked):
    found = set()
    for row in csv.reader(io.StringIO(tasklist_csv)):
        if row and row[0].lower() in blocked:
            found.add(row[0].lower())
    return sorted(found)


def competing_processes(blocked):
    output = subprocess.run(['tasklist', '/FO', 'CSV', '/NH'], capture_output=True, text=True,
                            encoding='oem', errors='replace', check=True).stdout
    return running_processes(output, blocked)


def describe_power(ac_line, battery_percent, status_flag):
    return {'ac_power': {0: False, 1: True}.get(ac_line),
            'battery_percent': None if battery_percent == 255 else battery_percent,
            'battery_saver': bool(status_flag & 1)}


class _PowerStatus(ctypes.Structure):
    _fields_ = [('ACLineStatus', ctypes.c_ubyte), ('BatteryFlag', ctypes.c_ubyte),
                ('BatteryLifePercent', ctypes.c_ubyte), ('SystemStatusFlag', ctypes.c_ubyte),
                ('BatteryLifeTime', ctypes.c_ulong), ('BatteryFullLifeTime', ctypes.c_ulong)]


def power_status():
    status = _PowerStatus()
    if not ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(status)):
        return {'ac_power': None, 'battery_percent': None, 'battery_saver': None}
    return describe_power(status.ACLineStatus, status.BatteryLifePercent,
                          status.SystemStatusFlag)


def describe_power_mode(ac_overlay, dc_overlay):
    def name(guid):
        return None if guid is None else POWER_MODES.get(guid.lower(), 'unknown')
    return {'ac_overlay': ac_overlay, 'ac_mode': name(ac_overlay),
            'dc_overlay': dc_overlay, 'dc_mode': name(dc_overlay)}


def power_mode():
    """Read the Windows power-mode overlay GUIDs from the registry (read-only)."""
    import winreg
    values = {}
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, OVERLAY_KEY) as key:
            for value in ('ActiveOverlayAcPowerScheme', 'ActiveOverlayDcPowerScheme'):
                try:
                    values[value] = winreg.QueryValueEx(key, value)[0]
                except FileNotFoundError:
                    values[value] = None
    except OSError:
        return describe_power_mode(None, None)
    return describe_power_mode(values['ActiveOverlayAcPowerScheme'],
                               values['ActiveOverlayDcPowerScheme'])


@contextmanager
def keep_awake():
    """Block idle sleep while this process runs; system power settings stay unchanged."""
    kernel32 = ctypes.windll.kernel32
    kernel32.SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)
    try:
        yield
    finally:
        kernel32.SetThreadExecutionState(ES_CONTINUOUS)


def hide_paths(text):
    """Mask the checkout and home directories so logs are safe to commit."""
    replacements = [(str(ROOT), '<repo>'), (ROOT.as_posix(), '<repo>'),
                    (str(Path.home()), '<home>'), (Path.home().as_posix(), '<home>')]
    for local, mask in sorted(replacements, key=lambda item: -len(item[0])):
        text = text.replace(local, mask)
    return text


def unique_path(path):
    candidate, index = path, 2
    while candidate.exists():
        candidate = path.with_name(f'{path.stem}-{index}{path.suffix}')
        index += 1
    return candidate
