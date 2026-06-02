# Deploying Wisecrow on calypso

The Wisecrow web app is a Dioxus fullstack server that terminates TLS itself and
serves an authenticated, multi-user study UI backed by PostgreSQL. This runbook
covers a production deployment to the `calypso` host.

## Architecture

- `wisecrow-web` — the fullstack server (axum + rustls); serves HTTPS on 8443.
- PostgreSQL 15 — corpus and per-user data.
- Both run as containers via `docker-compose.deploy.yml`. The `ansible/`
  playbooks sync the repo to the host, render the runtime `.env`, build the
  image, and bring the stack up.

There is no reverse proxy: the application owns TLS termination and certificate
loading.

## Prerequisites

- Docker and the Compose plugin on calypso.
- A TLS certificate and private key for the public hostname.
- Ansible on the control machine (to run the `ansible/` playbooks), or run
  `docker compose` directly on the host.

## 1. TLS certificates

Provide a PEM chain and key in the cert directory (`wisecrow_cert_dir`, default
`./certs` next to the compose file):

- `fullchain.pem` — the certificate chain.
- `privkey.pem` — the private key.

The container runs as uid 10001, so the files must be readable by it:

```sh
sudo chown root:10001 certs/privkey.pem && sudo chmod 0640 certs/privkey.pem
sudo chmod 0644 certs/fullchain.pem
```

Obtain certificates however you prefer (Let's Encrypt via certbot, an internal
CA, etc.). Renewal is the operator's responsibility — the server loads certs at
start-up, so restart the container after a renewal:

```sh
docker compose -f docker-compose.deploy.yml restart wisecrow-web
```

## 2. Secrets

Copy the example, fill it in, and vault-encrypt it:

```sh
cp ansible/vars/secrets.yml.example ansible/vars/secrets.yml
$EDITOR ansible/vars/secrets.yml
ansible-vault encrypt ansible/vars/secrets.yml
```

- Use **distinct** values for `postgres_password` and `become_pw_calypso` (the
  host sudo password). Never reuse one for the other.
- Treat any password that has been committed or shared in plaintext as
  compromised, and **rotate it** — change it in Postgres / on the host and
  update the vault.
- `sync_api_key_secret` is the optional legacy single sync key; prefer per-client
  keys (step 5).

## 3. Build and start

With ansible:

```sh
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/preflight.yml
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/install.yml --ask-vault-pass
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/start.yml
```

`install.yml` syncs the repo, renders `.env` (mode 0600), builds the image, and
brings the stack up; it waits for both containers to report healthy and runs a
`list-languages` smoke test.

Or directly on the host:

```sh
POSTGRES_PASSWORD=... docker compose -f docker-compose.deploy.yml up -d --build
```

## 4. Create the first admin

There is no public signup. Bootstrap the first admin account on the host once the
stack is up:

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  env WISECROW__INIT_PASSWORD='choose-a-strong-password' \
  wisecrow user add --email you@example.com --display-name "You" --admin
```

`WISECROW__INIT_PASSWORD` provisions non-interactively; omit it to be prompted.
Manage further accounts:

```sh
wisecrow user add --email user@example.com --display-name "User"
wisecrow user list
wisecrow user passwd --email user@example.com
wisecrow user disable --email user@example.com   # clears the password, revokes sessions
```

## 5. Sync-client keys (optional)

If another instance pulls the corpus from this one, provision a per-client key:

```sh
docker compose -f docker-compose.deploy.yml exec wisecrow-web \
  wisecrow sync-client add --name laptop   # prints the key once — store it
wisecrow sync-client list
wisecrow sync-client revoke --name laptop
```

The puller sends the key as the `x-api-key` header. Keys are individually
revocable and compared in constant time.

## 6. Verify

```sh
curl -sk https://<host>:8443/ -o /dev/null -w '%{http_code}\n'   # a response over TLS
docker compose -f docker-compose.deploy.yml ps                    # both healthy
```

Log in at `https://<host>:8443/login`.

## Configuration reference (environment)

| Variable | Purpose |
|---|---|
| `WISECROW__DB_*` | Database connection (set by compose from the secrets). |
| `WISECROW__TLS_CERT_PATH` / `WISECROW__TLS_KEY_PATH` | PEM cert/key paths inside the container (`/certs/*`). |
| `IP` / `PORT` | Bind address (default `0.0.0.0:8443`). |
| `WISECROW__LLM_PROVIDER` / `WISECROW__LLM_API_KEY` | LLM provider for gloss / graded-reader / quizzes. |
| `WISECROW__LLM_RATELIMIT_PER_MIN` | Per-user LLM request cap (default 20). |
| `WISECROW__SYNC_API_KEY` | Legacy single sync key (per-client keys preferred). |
| `RUST_LOG` / `RUST_BACKTRACE` | Logging (backtrace off by default in the image). |

## Updating

```sh
ansible-playbook -i ansible/inventory/hosts.yml ansible/playbooks/update.yml --ask-vault-pass
```

Database migrations apply automatically on start-up.
