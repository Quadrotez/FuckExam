from __future__ import annotations

import re
import shutil
import subprocess
from dataclasses import dataclass

from .models import VMInfo


class VirtualBoxError(RuntimeError):
    """Raised when VBoxManage is unavailable or returns an invalid result."""


@dataclass
class VBoxManageClient:
    executable: str | None = None
    timeout: float = 10.0

    def __post_init__(self) -> None:
        if self.executable is None:
            self.executable = shutil.which("VBoxManage") or shutil.which("VBoxManage.exe")
        if not self.executable:
            raise VirtualBoxError("VBoxManage не найден. Установите VirtualBox и добавьте VBoxManage в PATH.")

    def _run(self, *args: str) -> str:
        try:
            result = subprocess.run(
                [self.executable, *args],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=self.timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise VirtualBoxError(f"VBoxManage превысил тайм-аут: {' '.join(args)}") from exc
        if result.returncode != 0:
            details = result.stderr.strip() or result.stdout.strip() or "без подробностей"
            raise VirtualBoxError(f"VBoxManage {' '.join(args)} завершился с кодом {result.returncode}: {details}")
        return result.stdout

    def show_vm(self, identifier: str) -> VMInfo:
        raw = self._run("showvminfo", identifier, "--machinereadable")
        values: dict[str, str] = {}
        for line in raw.splitlines():
            if "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip('"')
        name = values.get("name")
        uuid = values.get("UUID")
        state = values.get("VMState", "unknown")
        if not name or not uuid:
            raise VirtualBoxError("VBoxManage вернул неполные данные VM (ожидались name и UUID).")
        return VMInfo(identifier=identifier, name=name, uuid=uuid, state=state)

    def list_vms(self) -> list[VMInfo]:
        raw = self._run("list", "vms")
        result: list[VMInfo] = []
        for line in raw.splitlines():
            match = re.match(r'^"(?P<name>.*)"\s+\{(?P<uuid>[^}]+)\}$', line.strip())
            if match:
                result.append(VMInfo(match.group("name"), match.group("name"), match.group("uuid"), "unknown"))
        return result
