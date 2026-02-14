# system-controller

A terminal user interface (TUI) for monitoring systemd services across remote hosts via SSH.


## Usage

The application requires two input files:

- **Inventory file** — an Ansible-style INI file listing target hosts
- **Services config** — a YAML file defining which systemd services to monitor


### Keyboard Controls

**Main screen:**

| Key     | Action                          |
|---------|---------------------------------|
| `Enter` | View details for selected service |
| `r`     | Refresh all statuses            |
| `c`     | open ssh connection to host     
| `s`     | stop service
| `t`     | restart service
| `q`     | Quit                            |

**Detail screen:**

A list of the commands or the files that can be viewed for that service.
If you select a file or command then it will open the output in the vim session
Same commands can be run in Detail Screen

### SSH Authentication

The application uses your existing SSH configuration (`~/.ssh/config`) and SSH agent for authentication. Ensure you can `ssh` to each host in your inventory without a password prompt before running.

### Example Files

`inventory.ini`:
```ini
[webservers]
192.168.1.10
192.168.1.11

[dbservers]
192.168.1.20
```

`services.yaml`:
```yaml
services:
  nginx:
    files:
      - /etc/nginx/nginx.conf
      - /var/log/nginx/error.log
    commands:
      - nginx -T
  postgresql:
    files:
      - /etc/postgresql/14/main/postgresql.conf
    commands:
      - pg_isready
  redis:
    commands:
      - redis-cli ping
  docker-*:
    commands:
      - docker stats --no-stream
```

Service names support glob patterns (`*`, `?`, `[`). On each host, patterns are matched against the available systemd units and expanded into individual rows. For example, `docker-*` on a host running `docker-api` and `docker-worker` produces two rows, each inheriting the configured `commands` and `files` from the pattern entry.

