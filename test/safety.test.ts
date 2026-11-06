import { describe, expect, test } from "bun:test"
import { needsPermission } from "../src/agent"
import { AUTO_SAFE, hardStop } from "../src/safety"
import { risk } from "../src/tools"

const bash = (command: string) => hardStop("bash", { command })

describe("auto mode safety", () => {
  test("hard stops catch catastrophic commands", () => {
    for (const c of ["rm -rf /", "rm -rf ~", "sudo rm -rf / --no-preserve-root", "rm -fr $HOME", "rm -rf /*", "mkfs.ext4 /dev/sda1", "dd if=/dev/zero of=/dev/sda", ":(){ :|:& };:", "chmod -R 777 /"])
      expect(bash(c)?.hard).toBe(true)
  })

  test("hard stops leave ordinary commands to the monitor", () => {
    for (const c of ["rm -rf node_modules", "rm -rf ./dist", "rm -r /tmp/oah-build", "git push", "dd if=a.img of=b.img", "chmod 755 script.sh"]) expect(bash(c)).toBeNull()
    expect(hardStop("write_file", { path: "/" })).toBeNull()
  })

  test("edits inside the project run; outside are reviewed", () => {
    const cwd = "/home/u/proj"
    expect(risk("edit_file", { path: "src/a.ts" }, cwd, [])).toBe("write")
    expect(risk("write_file", { path: "/etc/hosts" }, cwd, [])).toBe("write-outside")
    expect(risk("write_file", { path: "~/.bashrc" }, cwd, [])).toBe("write-outside")
    expect(risk("apply_patch", { patch: "*** Begin Patch\n*** Update File: src/a.ts\n*** End Patch" }, cwd, [])).toBe("write")
    expect(risk("apply_patch", { patch: "*** Begin Patch\n*** Update File: src/a.ts\n*** Move to: ../other/a.ts\n*** End Patch" }, cwd, [])).toBe("write-outside")
    expect(AUTO_SAFE.has("write")).toBe(true)
    expect(needsPermission("auto", "exec")).toBe(true)
    expect(needsPermission("auto", "external")).toBe(true)
  })
})
