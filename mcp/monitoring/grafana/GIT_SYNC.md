# Grafana Git Sync (kube-prometheus-stack)

This config makes Grafana continuously load dashboards from this Git repo.

## Files

- `monitoring/grafana/train-mcp-dashboard.json`
- `monitoring/grafana/kube-prometheus-stack-git-sync-values.yaml`

## Apply

```bash
helm upgrade --install kube-prometheus-stack prometheus-community/kube-prometheus-stack \
  --namespace monitoring --reuse-values \
  -f monitoring/grafana/kube-prometheus-stack-git-sync-values.yaml
```

## Verify

```bash
kubectl -n monitoring get pods -l app.kubernetes.io/name=grafana
kubectl -n monitoring logs deploy/kube-prometheus-stack-grafana -c git-sync --tail=80
kubectl -n monitoring exec deploy/kube-prometheus-stack-grafana -c grafana -- \
  ls -la /var/lib/grafana/dashboards/git/current/monitoring/grafana
```

Then open Grafana and check folder **Train MCP**.

## Private repo notes

If your repo is private, add Git credentials as env vars in `extraContainers`:

- `GITSYNC_USERNAME`
- `GITSYNC_PASSWORD` (from a Secret)

or switch to SSH mode with mounted SSH key and known hosts.
