---
name: Bug report
about: Something does not work or looks wrong
labels: bug
---

**What happened, and what did you expect?**

**Steps to reproduce**

1.
2.

**Your system**

- Laptop or desktop, CPU, GPU(s):
- Fedora and GNOME version:
- Andromeda version (Settings → About):

**Logs**

```
journalctl --user -b | grep -i andromeda
```

<details><summary>Sensors (only for wrong or missing readings)</summary>

Output of `cargo run -p andromeda-core --example probe` from a source checkout.

</details>
