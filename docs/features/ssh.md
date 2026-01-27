# Remote Editing (Experimental)

Fresh supports editing files on remote machines via SSH using the `user@host:path` syntax. This is useful for editing files on servers without needing to install Fresh remotely.

```bash
# Open a specific file
fresh deploy@server.example.com:/etc/nginx/nginx.conf

# Open home directory in file explorer
fresh user@host:~

# Open with line number
fresh user@host:/var/log/app.log:100
```

**Features:**
- Password and SSH key authentication
- File explorer shows remote directory
- Sudo save support for protected files
- Status bar shows `[SSH:user@host]` indicator

**Requirements:**
- SSH access to the remote host
- Python 3 installed on the remote host (for the agent)
