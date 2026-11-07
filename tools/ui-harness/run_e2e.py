#!/usr/bin/env python3
"""End-to-end UI tests: the real UI (built by Trunk) in headless Chrome,
with the backend replaced by the harness stubs in this folder. Each scenario
loads one stub with a query, lets the stub click through a flow, and checks
the page: the stubs put what the app sent to the backend (or the state they
observed) in the page title, and the rest of the page is checked as text.

    python3 tools/ui-harness/run_e2e.py              # build, then run everything
    python3 tools/ui-harness/run_e2e.py helm config  # only scenarios whose id contains a word
    python3 tools/ui-harness/run_e2e.py --show helm/rollback-default   # print what a scenario saw

Needs Google Chrome and Trunk; macOS paths by default (set CHROME to override).
"""

import concurrent.futures
import functools
import html
import json
import http.server
import os
import re
import shutil
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
HARNESS = os.path.join(ROOT, "tools", "ui-harness")
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
LIGHT, DARK = 1, 0


def S(id, stub, query, has=(), lacks=(), scheme=LIGHT, budget=8000, real_time=False):
    """A scenario. `has`/`lacks`: strings that must (not) appear in the title
    or page text; a string starting with `re:` is a regular expression."""
    return dict(id=id, stub=stub, query=query, has=list(has), lacks=list(lacks), scheme=scheme, budget=budget, real_time=real_time)


SCENARIOS = [
    # ---- Pods (stub.js): details, delete, upload, download ----
    S("pod/details", "stub.js", "pod=limited", has=["Resources", "QoS class", "Burstable", "Forward", "Upload", "Shell", "Logs", "Running containers 2"],
      lacks=["Init containers", "Ephemeral containers"]),
    # Containers by group: running (with the sidecar) open, init and ephemeral closed until opened.
    S("pod/container-groups", "stub.js", "pod=groups",
      has=["Running containers 2", "proxy", "Sidecar ·", "Init containers 1", "Ephemeral containers 1"], lacks=["ghcr.io/acme/migrate:1", "ghcr.io/acme/netshoot:1"]),
    S("pod/container-groups-init", "stub.js", "pod=groups&opengroup=init", has=["migrate", "ghcr.io/acme/migrate:1", "Completed"], lacks=["ghcr.io/acme/netshoot:1"]),
    S("pod/container-groups-ephemeral", "stub.js", "pod=groups&opengroup=ephemeral", has=["subduer-capture-1", "ghcr.io/acme/netshoot:1"]),
    # No charts or environment for ephemeral containers: the app's containers have two each.
    S("pod/container-groups-ephemeral-plain", "stub.js", "pod=groups&opengroup=ephemeral", has=["re:(Environment.*){2}"], lacks=["re:(Environment.*){3}"]),
    S("pod/container-groups-dark", "stub.js", "pod=groups&opengroup=ephemeral", scheme=DARK, has=["Ephemeral containers"]),
    # Capture traffic…: the dialog sends the request; Start needs an image and changes allowed.
    S("capture/dialog", "stub.js", "pod=limited&cap=1",
      has=["Capture traffic", "re:All traffic \\(TCP\\).*Inbound to port 8080 \\(http\\).*Custom filter.*TLS handshakes only", "5 min", "stays listed in the pod"],
      lacks=["start_capture", "Inbound to port 8080 (http) Inbound to port 8080"]),
    S("capture/start", "stub.js", "pod=limited&cap=1&capimg=acme/netshoot:1&capgo=1",
      has=['start_capture {"namespace":"apps","pod":"acme-limited","image":"acme/netshoot:1","filter":{"traffic":"All","handshakes_only":true},"secs":300,'
           '"path":"/Users/me/Documents/acme-uat-apps-acme-limited-']),
    # The Save dialog comes first; cancelling it stays in the capture dialog.
    S("capture/save-cancelled", "stub.js", "pod=limited&cap=1&capimg=x&capgo=1&capcancel=1",
      has=["choose_capture_file acme-uat-apps-acme-limited-", "Capture traffic", "Save as and start…"], lacks=["start_capture"]),
    S("capture/remembered-image", "stub.js", "pod=limited&cap=1&capsaved=ecr/netshoot:2&capsecs=60&capgo=1",
      has=['"image":"ecr/netshoot:2","filter":{"traffic":"All","handshakes_only":true},"secs":60']),
    S("capture/inbound-port", "stub.js", "pod=limited&cap=1&capimg=x&capfilter=port&capgo=1", has=['"filter":{"traffic":{"Inbound":8080},"handshakes_only":true}']),
    S("capture/everything", "stub.js", "pod=limited&cap=1&capimg=x&caphs=0&capgo=1",
      has=['"filter":{"traffic":"All","handshakes_only":false}']),
    S("capture/everything-warns", "stub.js", "pod=limited&cap=1&capimg=x&caphs=0", has=["Without it, the capture holds any plain-text data"]),
    S("capture/no-ports", "stub.js", "pod=unlimited&noports=1&cap=1", has=["All traffic (TCP)"], lacks=["Inbound to port"]),
    S("capture/custom-filter", "stub.js", "pod=limited&cap=1&capimg=x&capfilter=custom&capexpr=port 443&capgo=1", has=['"filter":{"traffic":{"Custom":"port 443"},"handshakes_only":true}']),
    S("capture/needs-image", "stub.js", "pod=limited&cap=1&capgo=1", has=["Capture traffic"], lacks=["start_capture"]),
    S("capture/locked", "stub.js", "pod=limited&locked=1&cap=1&capimg=x&capgo=1", has=["is read-only", "Allow changes…"], lacks=["start_capture"]),
    # Production asked for its name when changes were allowed; a capture is a plain OK.
    S("capture/prod-no-name", "stub.js", "pod=limited&ctx=prod&cap=1&capimg=x&capgo=1", has=["start_capture", "acme-prd"]),
    S("capture/refused", "stub.js", "pod=limited&cap=1&capimg=x&capgo=1&capfail=1", has=["cannot patch resource", "Capture traffic"]),
    S("capture/dialog-dark", "stub.js", "pod=limited&cap=1", scheme=DARK, has=["Capture traffic"]),
    # The capture window, following a capture: starting, live, finished, failed, stopped early.
    S("capture/window-starting", "capturestub.js", "capture=capture-1&slow=1",
      has=["Starting the capture container", "acme-limited", "subduer-capture-1791804344", "No TLS handshakes seen yet", "ecr.example/netshoot:latest"],
      lacks=["sha256"]),
    S("capture/window-live", "capturestub.js", "capture=capture-1",
      has=["Capturing", "left", "4 TLS connections", "api.example.com", "TLS 1.3 · 3", "no answer · 1", "netshoot@sha256:e47b97ba", "Stop", "59.8 KiB"]),
    S("capture/window-old-version", "capturestub.js", "capture=capture-1&old=1", has=["TLS 1.2 · 1", "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"]),
    S("capture/window-done", "capturestub.js", "capture=capture-1&end=done", has=["Finished", "40 packets captured"], lacks=["Stop"]),
    S("capture/window-failed", "capturestub.js", "capture=capture-1&end=fail", has=["Failed", "ErrImagePull: 403 Forbidden"], lacks=["Stop"]),
    S("capture/window-stop", "capturestub.js", "capture=capture-1&stop=1", has=["sent=capture_session,stop_capture", "Stopping…"]),
    S("capture/window-dark", "capturestub.js", "capture=capture-1&end=done", scheme=DARK, has=["Finished"]),
    S("pod/details-dark", "stub.js", "pod=unlimited", scheme=DARK, has=["Details", "YAML"]),
    # ---- Navigation ----
    # Section headings are uppercase on screen only (CSS); the text is title case.
    S("nav/sidebar", "nav.js", "open=all", has=["Workloads", "Config", "Network", "Storage", "Port Forwards", "Cluster"], lacks=["Workloads 7", "Config 9", "Network 5", "Storage 3"]),
    S("nav/forwards", "fwd.js", "page=forwards", has=["Port Forwards"]),
    S("sort/table-number", "nav.js", "sort=Revision", has=["rows keda,datadog,migrate,canary,corporate-wallet"]),
    S("sort/table-number-desc", "nav.js", "sort=Revision&clicks=2", has=["rows corporate-wallet,canary,migrate,datadog,keda"]),
    S("sort/table-age", "nav.js", "sort=Updated", has=["rows canary,corporate-wallet,migrate,datadog,keda"]),
    S("sort/table-name", "nav.js", "sort=Name", has=["rows canary,corporate-wallet,datadog,keda,migrate"]),
    S("sort/table-off", "nav.js", "sort=Name&clicks=3", has=["rows datadog,keda,corporate-wallet,migrate,canary"]),
    # ---- Deployments / StatefulSets ----
    S("workload/details", "workloadstub.js", "rolling=1", has=["Rolling out", "Pods", "Conditions", "Resources"]),
    S("workload/restart", "workloadstub.js", "op=restart&go=1", has=["restart_workload", '"name":"acme-api"']),
    S("workload/scale", "workloadstub.js", "op=scale&to=5&go=1", has=["scale_workload", '"replicas":5']),
    S("workload/prod-no-name", "workloadstub.js", "op=restart&ctx=prod&go=1", has=["restart_workload", '"context":"acme-prd"'], lacks=["Type the name"]),
    S("workload/statefulset", "workloadstub.js", "sts=1", has=["StatefulSet", "acme-db"]),
    # ---- Jobs / CronJobs ----
    S("batch/cronjob", "batchstub.js", "", has=["Jobs", "Next run", "manual", "Run now"]),
    S("batch/run-now", "batchstub.js", "op=run&go=1", has=["run_cron_job", '"name":"nightly"']),
    S("batch/suspend-cron", "batchstub.js", "op=suspend&go=1", has=["suspend_workload", '"suspend":true']),
    S("batch/resume-cron", "batchstub.js", "op=resume&suspended=1&go=1", has=["suspend_workload", '"suspend":false']),
    S("batch/suspend-job", "batchstub.js", "job=1&op=suspend&go=1", has=["suspend_workload", '"kind":"Job"']),
    S("batch/failed-job", "batchstub.js", "job=1&failed=1", has=["Failed", "took 30m 0s"], lacks=["Suspend"]),
    S("batch/open-job", "batchstub.js", "openjob=1", has=["opened jobs list"]),
    # ---- ConfigMaps / Secrets ----
    S("config/configmap", "configstub.js", "kind=cm", has=["Data", "LOG_LEVEL", "Used by", "Managed by Helm"]),
    S("config/secret-masked", "configstub.js", "kind=secret", lacks=["hunter2", "tok_8f2a9c1e77"], has=["Show"]),
    S("config/secret-show", "configstub.js", "kind=secret&show=1", has=["tok_8f2a9c1e77"], lacks=["hunter2"]),
    S("config/save-configmap", "configstub.js", "kind=cm&edit=1&change=1&add=1&remove=1&save=1&go=1",
      has=["save_config_data", '"resource_version":"4711"', '"key":"FEATURE_FLAGS"', '"key":"LOG_LEVEL","value":null']),
    S("config/save-prod", "configstub.js", "kind=cm&ctx=prod&edit=1&change=1&save=1&go=1",
      has=["save_config_data", '"context":"acme-prd"']),
    S("config/save-secret", "configstub.js", "kind=secret&edit=1&change=1&save=1&go=1",
      has=["save_config_data", '"key":"API_TOKEN","value":"tok_rotated_2026"']),
    S("config/bad-key", "configstub.js", "kind=cm&edit=1&badkey=1", has=["isn't a valid key"]),
    S("config/locked-save", "configstub.js", "kind=cm&locked=1&edit=1&change=1&save=1", has=["Allow changes?"]),
    S("config/hpa-range", "configstub.js", "kind=hpa&range=1&go=1", has=["set_hpa_range", '"min":2', '"max":14']),
    S("config/quota", "configstub.js", "kind=quota", has=["At limit", "requests.memory"]),
    S("config/webhook", "configstub.js", "kind=webhook", has=["check.acme.io", "Fail: requests are rejected"]),
    S("config/pdb", "configstub.js", "kind=pdb", has=["Blocking"]),
    S("config/lease", "configstub.js", "kind=lease", has=["Held", "Changed hands"]),
    # ---- Unsaved edits: every way out asks, Keep stays, Discard leaves ----
    *[
        scenario
        for leave in ["row", "close", "yaml", "kind", "refresh", "link", "ns", "cancel", "window", "quit"]
        for scenario in [
            S(f"unsaved/{leave}-asks", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}",
              has=["prompt=true", "editing=true", "unsaved=true"]),
            S(f"unsaved/{leave}-keep", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}&keep=1",
              has=["prompt=false", "editing=true", "unsaved=true"]),
            S(f"unsaved/{leave}-discard", "configstub.js", f"kind=cm&edit=1&change=1&leave={leave}&discard=1",
              has=["prompt=false", "editing=false", "unsaved=false"]),
        ]
    ],
    S("unsaved/clean-edit-leaves", "configstub.js", "kind=cm&edit=1&leave=row", has=["prompt=false", "selected=othershop"]),
    S("unsaved/window-discard-closes", "configstub.js", "kind=cm&edit=1&change=1&leave=window&discard=1", has=["closed=window"]),
    S("unsaved/quit-discard-quits", "configstub.js", "kind=cm&edit=1&change=1&leave=quit&discard=1", has=["closed=quit"]),
    # ---- Network ----
    S("network/service", "networkstub.js", "kind=svc", has=["Partly ready", "Endpoints", "Routed from"]),
    S("network/ingress", "networkstub.js", "kind=ingress", has=["Backend missing", "no such service", "URLs"]),
    S("network/open-url", "networkstub.js", "kind=ingress&open=1", has=['open_url {"url":"https://shop.acme.io/"}']),
    S("network/netpol", "networkstub.js", "kind=netpol", has=["Denies outgoing", "Pods it applies to"]),
    S("network/slice", "networkstub.js", "kind=slice", has=["2 of 3 ready"]),
    # ---- Storage ----
    S("storage/pvc", "storagestub.js", "kind=pvc", has=["Disk usage", "87% used", "Mounted by", "Expand"]),
    S("storage/pending", "storagestub.js", "kind=pvc&pending=1", has=["Why pending", "WaitForFirstConsumer"]),
    S("storage/expand", "storagestub.js", "kind=pvc&op=expand&size=40Gi&go=1", has=["expand_claim", '"size":"40Gi"']),
    S("storage/expand-smaller", "storagestub.js", "kind=pvc&op=expand&size=10Gi&go=1", lacks=["expand_claim"]),
    S("storage/reclaim-prod", "storagestub.js", "kind=pv&op=reclaim&ctx=prod&go=1",
      has=["set_reclaim_policy", '"policy":"Retain"', '"namespace":""']),
    S("storage/class", "storagestub.js", "kind=sc", has=["Default", "Claims", "Parameters"]),
    S("storage/attachment", "storagestub.js", "kind=va", has=["Error", "Attach error"]),
    S("sort/details-asc", "storagestub.js", "kind=sc&sort=Status", has=["rows type,encrypted,shop,shop,ops", "Status ↑"]),
    S("sort/details-ns", "storagestub.js", "kind=sc&sort=Namespace", has=["rows type,encrypted,ops,shop,shop"]),
    S("sort/details-desc", "storagestub.js", "kind=sc&sort=Namespace&clicks=2", has=["rows type,encrypted,shop,shop,ops", "Namespace ↓"]),
    S("sort/details-claim-desc", "storagestub.js", "kind=sc&sort=Claim&clicks=2", has=["rows type,encrypted,ops,shop,shop"]),
    S("sort/details-off", "storagestub.js", "kind=sc&sort=Claim&clicks=3", has=["rows type,encrypted,shop,shop,ops"], lacks=["Claim ↑", "Claim ↓"]),
    # ---- External Secrets ----
    S("eso/external-secret", "esostub.js", "kind=es",
      has=["Synced", "SecretStore/default-store", "Target secret", "Next refresh", "Data from", "acme/uat/api-key", "secret updated", "Force sync", "Controller logs"]),
    S("eso/failed", "esostub.js", "kind=es&failed=1", has=["Sync failed", "AccessDeniedException", "UpdateFailed"]),
    S("eso/force", "esostub.js", "kind=es&op=force&go=1",
      has=["force_sync", '"namespace":"shop"', '"name":"acme-api"', '"kind":"ExternalSecret"']),
    S("eso/force-dialog", "esostub.js", "kind=es&op=force", has=["Force sync ExternalSecret?", "force-sync", "fetches from the store"], lacks=["CreatedOnce"]),
    S("eso/force-created-once", "esostub.js", "kind=es&once=1&op=force", has=["CreatedOnce: External Secrets fetched it once"]),
    S("eso/force-prod", "esostub.js", "kind=es&op=force&ctx=prod&go=1", has=["force_sync", '"context":"acme-prd"']),
    S("eso/force-locked", "esostub.js", "kind=es&op=force&locked=1&go=1", has=["force_sync"]),
    S("eso/cluster", "esostub.js", "kind=ces", has=["Partly failed", "labelled team=shop", "Failed: namespace is being terminated", "Force sync"]),
    S("eso/cluster-force", "esostub.js", "kind=ces&op=force&go=1", has=["force_sync", '"namespace":""', '"kind":"ClusterExternalSecret"']),
    S("eso/cluster-force-dialog", "esostub.js", "kind=ces&op=force", has=["do its 2 ExternalSecrets"]),
    S("eso/store", "esostub.js", "kind=store", has=["Valid", "AWS Secrets Manager (us-east-1)", "Provider: aws", "Used by", "acme-worker"], lacks=["Force sync"]),
    S("eso/generator", "esostub.js", "kind=gen", has=["length", "No ExternalSecrets use it.", "Controller logs"], lacks=["Force sync"]),
    S("eso/logs", "esostub.js", "kind=es&ctl=1", has=["open_logs", '"filter":"acme-api"', '"name":"external-secrets"']),
    S("eso/link", "esostub.js", "kind=es&link=1", has=["details SecretStore default-store"]),
    # ---- KEDA ----
    S("keda/scaled-object", "kedastub.js", "kind=so",
      has=["Active", "Deployment acme-api", "3 now · 4 wanted", "keda-hpa-acme-api", "When idle", "Triggers", "61 average", "70% of request",
           "TriggerAuthentication/prom-auth", "Trigger: prometheus (rps)", "(not shown)", "Started scalers watch", "Controller logs"], lacks=["Force sync"]),
    S("keda/failing", "kedastub.js", "kind=so&failed=1", has=["Not ready", "failing (4 in a row)", "KEDAScalerFailed", "connection refused"]),
    S("keda/paused", "kedastub.js", "kind=so&paused=1", has=["Paused", "none while paused", "autoscaling.keda.sh/paused"], lacks=["61 average", "3 now"]),
    S("keda/scaled-job", "kedastub.js", "kind=sj", has=["Idle", "worker (acme/worker:1)", "Jobs at once", "gradual", "acme-worker-x7k2p", "3 · 1 running", "amqp://acme:(not shown)@"]),
    S("keda/auth", "kedastub.js", "kind=auth", has=["Parameters", "prom-creds", "PROM_USER", "Used by", "acme-search", "Not ready", "Role ARN"]),
    S("keda/cluster-auth", "kedastub.js", "kind=cauth", has=["keda (KEDA's namespace)", "prom-creds"]),
    S("keda/logs", "kedastub.js", "kind=so&ctl=1", has=["open_logs", '"filter":"acme-api"', '"name":"keda-operator"']),
    S("keda/link", "kedastub.js", "kind=so&link=1", has=["details HorizontalPodAutoscaler keda-hpa-acme-api"]),
    S("keda/dark", "kedastub.js", "kind=so&failed=1", scheme=DARK, has=["Not ready", "Triggers"]),
    S("keda/buttons", "kedastub.js", "kind=so", has=["Edit range", "Pause", "Controller logs"], lacks=["Resume"]),
    S("keda/pause-dialog", "kedastub.js", "kind=so&op=pause", has=["Pause ScaledObject?", "stops scaling", "it stays at 3 replicas", "Scale it to a replica count first"]),
    S("keda/pause", "kedastub.js", "kind=so&op=pause&go=1", has=["pause_scaling", '"paused":true', '"name":"acme-api"'], lacks=['"replicas"']),
    S("keda/pause-hold", "kedastub.js", "kind=so&op=pause&hold=0&go=1", has=["pause_scaling", '"replicas":0', '"paused":true']),
    S("keda/pause-hold-dialog", "kedastub.js", "kind=so&op=pause&hold=0", has=["Pause at 0", "paused-replicas to 0"]),
    S("keda/resume", "kedastub.js", "kind=so&paused=1&op=resume&go=1", has=["pause_scaling", '"paused":false']),
    S("keda/resume-button", "kedastub.js", "kind=so&paused=1", has=["Resume", "Edit range"]),
    S("keda/job-pause", "kedastub.js", "kind=sj&op=pause", has=["Pause ScaledJob?", "Jobs already running carry on"], lacks=["Scale it to a replica count first"]),
    S("keda/range-dialog", "kedastub.js", "kind=so&op=range", has=["Edit replica range?", "Set 0–10", "scales it to zero"]),
    S("keda/range", "kedastub.js", "kind=so&op=range&min=1&max=12&go=1", has=["set_scaling_range", '"min":1', '"max":12', '"kind":"ScaledObject"']),
    S("keda/range-invalid", "kedastub.js", "kind=so&op=range&min=5&max=2", has=["Min can't be below 0"]),
    S("keda/range-unchanged", "kedastub.js", "kind=so&op=range&go=1", lacks=["set_scaling_range"]),
    S("keda/range-helm", "kedastub.js", "kind=so&helm=1&op=range", has=["Managed by Helm (release acme-api)", "helm upgrade"]),
    S("keda/range-not-helm", "kedastub.js", "kind=so&op=range", lacks=["Managed by Helm"]),
    S("keda/range-paused", "kedastub.js", "kind=so&paused=1&op=range", has=["applies once it's resumed"]),
    S("keda/job-range", "kedastub.js", "kind=sj&op=range", has=["Edit Jobs at once?", "Set 0–20"]),
    S("keda/job-range-go", "kedastub.js", "kind=sj&op=range&max=30&go=1", has=["set_scaling_range", '"max":30', '"kind":"ScaledJob"']),
    S("keda/range-prod", "kedastub.js", "kind=so&op=range&max=12&ctx=prod&go=1", has=["set_scaling_range", '"context":"acme-prd"']),
    S("keda/pause-locked", "kedastub.js", "kind=so&op=pause&locked=1&go=1", has=["pause_scaling"]),
    # ---- Access control ----
    S("rbac/serviceaccount", "rbacstub.js", "kind=sa", has=["Sensitive", "What it can do", "reaches the kubelet", "Check access"]),
    S("rbac/check-allowed", "rbacstub.js", "kind=sa&check=pods", has=["check_access", "Allowed"]),
    S("rbac/check-denied", "rbacstub.js", "kind=sa&check=secrets", has=["Denied"]),
    S("rbac/role", "rbacstub.js", "kind=role", has=["Bound by", "reads secrets"]),
    S("rbac/binding", "rbacstub.js", "kind=binding", has=["Everyone", "What it grants"]),
    # ---- Helm ----
    S("helm/release", "helmstub.js", "", has=["History", "Resources", "Notes", "Rollback", "Delete"]),
    S("helm/rollback-default", "helmstub.js", "op=rollback&go=1", has=["helm_rollback", '"revision":40', '"wait":true']),
    S("helm/rollback-pick", "helmstub.js", "op=rollback&to=39&nowait=1&go=1", has=['"revision":39', '"wait":false']),
    S("helm/rollback-failed-warns", "helmstub.js", "op=rollback&to=41", has=["That revision failed"]),
    S("helm/rollback-prod", "helmstub.js", "op=rollback&ctx=prod&go=1", has=["helm_rollback"]),
    S("helm/rollback-error", "helmstub.js", "op=rollback&go=1&fail=1", has=["timed out waiting"]),
    S("helm/delete-needs-name", "helmstub.js", "op=delete&go=1", lacks=["helm_uninstall"]),
    S("helm/delete", "helmstub.js", "op=delete&typed=1&go=1", has=["helm_uninstall", '"keep_history":false']),
    S("helm/delete-keep-history", "helmstub.js", "op=delete&keep=1&typed=1&go=1", has=['"keep_history":true']),
    S("helm/cluster-scoped-link", "helmstub.js", "link=1", has=["selected=acme-api-reader"]),
    # ---- Namespaces ----
    S("ns/details", "nsstub.js", "", has=["Contents", "Recent warnings", "Quota on requests", "Filter to it"]),
    S("ns/stuck", "nsstub.js", "stuck=1", has=["Terminating", "widgets.acme.io"]),
    S("ns/create", "nsstub.js", "new=1&name=team-sandbox&go=1", has=['create_namespace {"context":"acme-uat","name":"team-sandbox"}']),
    S("ns/create-bad-name", "nsstub.js", "new=1&name=Bad_Name&go=1", lacks=["create_namespace"], has=["Use lowercase letters"]),
    S("ns/create-existing", "nsstub.js", "new=1&name=ops&go=1", lacks=["create_namespace"], has=["already exists"]),
    S("ns/create-after-unlock", "nsstub.js", "new=1&locked=1&name=team-sandbox&go=1", has=["create_namespace"]),
    S("ns/contents-link", "nsstub.js", "link=1", has=["list=Deployment ns=shop"]),
    S("ns/filter", "nsstub.js", "filter=1", has=["ns-menu=shop"]),
    # ---- Nodes (nodestub.js): Details tab, the pods on it, click-through to a pod ----
    S("node/details", "nodestub.js", "",
      has=["object_details Node node-a", "node_history node-a", "Details", "Resources", "3 of 29 pods", "req 1.8 cores (91%)",
           "allocatable 1.9 cores", "Last hour from Prometheus", "3 pods · 1 not ready", "proxy-1", "api-2",
           "CrashLoopBackOff", "Internal IP", "dedicated=batch:NoSchedule", "Events", "NodeNotReady"],
      lacks=["with namespace", "Capacity"]),
    # Resources comes first, before the node's own section.
    S("node/resources-first", "nodestub.js", "", has=["re:Resources.*Node\\s+Ready.*Status"]),
    # Fargate: one pod per node by design, so no pod count, and a note saying why.
    S("node/fargate", "nodestub.js", "fargate=1",
      has=["Pod count not shown: AWS Fargate runs one pod per node", "re:Resources\\s+AWS Fargate"], lacks=["1 of 1 pods", "re:Pods\\s+1 · 100%"]),
    S("node/local-history", "nodestub.js", "local=1", has=["the app records usage itself"]),
    S("node/cordoned", "nodestub.js", "cordoned=1", has=["Cordoned", "no new pods are placed here"]),
    S("node/not-ready", "nodestub.js", "notready=1", has=["NotReady", "unhealthy"]),
    S("node/pod-link", "nodestub.js", "link=1", has=["list=Pod", "pod_details shop/api-2"]),
    S("node/details-dark", "nodestub.js", "", scheme=DARK, has=["Resources", "api-2"]),
    # ---- Traces ----
    S("trace/button-idle", "tracestub.js", "", has=["Trace…", "indicator=none"]),
    S("trace/start-dialog", "tracestub.js", "open=1", has=["dialogs=Start a trace", "Whole cluster", "Every pod in acme-uat"]),
    S("trace/start-dialog-dark", "tracestub.js", "open=1&level=workloads&ns=shop", scheme=DARK, has=["Start a trace", "nightly", "CronJob"]),
    S("trace/menu", "tracestub.js", "menu=1", has=["dialogs=Start a trace"]),
    S("trace/start-cluster", "tracestub.js", "open=1&go=1",
      has=['choose_trace_file {"suggested":"acme-uat-cluster-', 'start_trace {"context":"acme-uat","scope":"Cluster","path":"/Users/me/Documents/acme-uat-cluster-',
           "dialogs=none", "indicator=Trace · 0m"]),
    S("trace/save-cancelled", "tracestub.js", "open=1&go=1&cancel=1", has=["choose_trace_file", "dialogs=Start a trace"], lacks=["start_trace"]),
    S("trace/namespaces", "tracestub.js", "open=1&level=ns&pick=ops&go=1", has=['"scope":{"Namespaces":["ops"]}', '"suggested":"acme-uat-ops-']),
    S("trace/namespaces-need-one", "tracestub.js", "open=1&level=ns&go=1", has=["0 chosen"], lacks=["choose_trace_file"]),
    S("trace/workloads", "tracestub.js", "open=1&level=workloads&ns=shop&pick=web&go=1",
      has=['trace_targets {"namespace":"shop"}', '"scope":{"Workloads":[{"kind":{"group":"apps","version":"v1","kind":"Deployment"', '"name":"web"}]}']),
    S("trace/pod", "tracestub.js", "open=1&level=pod&ns=shop&pick=db-0&go=1", has=['"scope":{"Pod":{"namespace":"shop","name":"db-0"}}']),
    S("trace/this-pod", "tracestub.js", "this=1&go=1", has=['"scope":{"Pod":{"namespace":"shop","name":"web-7d9f-abcde"}}']),
    S("trace/indicator", "tracestub.js", "running=1", has=["indicator=Trace · 2h 14m connects"]),
    S("trace/panel", "tracestub.js", "running=1&panel=1",
      has=["dialogs=Trace running", "namespace shop", "Last pulled from Prometheus 3m ago", "acme-uat-shop-2026-10-11-1405.trace", "1 running pod isn't in Prometheus"]),
    S("trace/panel-dark", "tracestub.js", "running=1&panel=1", scheme=DARK, has=["Trace running"]),
    S("trace/panel-warns", "tracestub.js", "running=1&panel=1&err=1&gaps=2", has=["The last pull failed", "2 gaps", "indicator=Trace · 2h 14m (warn)"]),
    S("trace/panel-no-prometheus", "tracestub.js", "running=1&panel=1&noprom=1", has=["No Prometheus found"]),
    S("trace/stop", "tracestub.js", "running=1&stop=1",
      has=['stop_trace {"reason":"Stopped"}', "dialogs=Trace saved", "21 pods, 29 containers", "indicator=none", "Show in Finder"]),
    S("trace/quit-asks", "tracestub.js", "running=1&quit=1", has=["dialogs=A trace is running", "Stop trace and quit"], lacks=["close_discarding"]),
    S("trace/quit-stops", "tracestub.js", "running=1&quit=1&confirm=1", has=['close_discarding {"quit":true}']),
    S("trace/quit-keep", "tracestub.js", "running=1&quit=1&keep=1", has=["dialogs=none", "indicator=Trace"], lacks=["close_discarding"]),
    S("trace/close-asks", "tracestub.js", "running=1&close=1", has=["Stop trace and close"]),
    S("trace/quit-without-trace", "tracestub.js", "quit=1", has=['close_discarding {"quit":true}', "dialogs=none"]),
    S("trace/switch-asks", "tracestub.js", "running=1&switch=1", has=["dialogs=Stop the trace?", "connects=1"]),
    S("trace/switch-stops", "tracestub.js", "running=1&switch=1&confirm=1",
      has=['stop_trace {"reason":"ContextChanged"}', "connects=2", "dialogs=Trace saved"]),
    S("trace/switch-keep", "tracestub.js", "running=1&switch=1&keep=1", has=["connects=1", "dialogs=none"], lacks=["stop_trace"]),
    S("trace/start-x-closes", "tracestub.js", "open=1&x=1", has=["dialogs=none"], lacks=["choose_trace_file"]),
    S("trace/start-escape-closes", "tracestub.js", "open=1&esc=1", has=["dialogs=none"]),
    S("trace/panel-x-closes", "tracestub.js", "running=1&panel=1&x=1", has=["dialogs=none", "indicator=Trace"], lacks=["stop_trace"]),
    S("trace/saved-escape-closes", "tracestub.js", "running=1&stop=1&esc=1", has=["dialogs=none", "stop_trace"]),
    # How often to pull: 10 min unless chosen; the last choice is remembered.
    S("trace/pull-every-default", "tracestub.js", "open=1&go=1", has=['"pull_every":600']),
    S("trace/pull-every-chosen", "tracestub.js", "open=1&every=1800&go=1&panel=1", has=['"pull_every":1800', "Every 30 min"]),
    S("trace/pull-every-remembered", "tracestub.js", "open=1&saved=1200&go=1", has=['"pull_every":1200']),
    S("trace/pull-every-unknown-saved", "tracestub.js", "open=1&saved=300&go=1", has=['"pull_every":600']),
    S("trace/saved-shows-pulled", "tracestub.js", "running=1&stop=1", has=["Pulled", "1.5 MiB from Prometheus"]),
    # ---- Trace review (reviewstub.js) ----
    S("review/empty-page", "reviewstub.js", "page=1", has=["page=Traces", "No traces yet", "Open trace…"]),
    S("review/recent-and-running", "reviewstub.js", "page=1&recent=1&running=1",
      has=["acme-uat-shop-2026-10-08-0900.trace", "acme-uat · Deployment web", "Recording namespace shop of acme-uat", "Review so far"]),
    S("review/open-file", "reviewstub.js", "page=1&open=1",
      has=["sent=trace_status,choose_trace_to_open,trace_review", "shared-acme-uat.trace", "rows=web,nightly,worker,db"]),
    S("review/open-cancelled", "reviewstub.js", "page=1&open=1&cancel=1", has=["page=Traces", "rows=none"], lacks=["trace_review"]),
    S("review/not-a-trace", "reviewstub.js", "page=1&open=1&bad=1", has=["isn't a trace file"]),
    S("review/table", "reviewstub.js", "recent=1&file=1",
      has=["rows=web,nightly,worker,db", "short 1.2 cores", "15m · 2.9 GiB", "OOMKilled ×2", "+4", "Covers", "3d 1h", "select one for its charts"]),
    S("review/table-dark", "reviewstub.js", "recent=1&file=1", scheme=DARK, has=["rows=web,nightly,worker,db"]),
    S("review/offline", "reviewstub.js", "recent=1&file=1&offline=1", has=["rows=web,nightly,worker,db"]),
    S("review/sort-by-peak", "reviewstub.js", "recent=1&file=1&sort=Memory peak", has=["rows=web,worker,db,nightly"]),
    S("review/sort-twice", "reviewstub.js", "recent=1&file=1&sort=CPU spare", has=["rows=worker,nightly,web,db"]),
    S("review/filter-namespace", "reviewstub.js", "recent=1&file=1&ns=ops", has=["rows=nightly"]),
    S("review/filter-text", "reviewstub.js", "recent=1&file=1&text=WOR", has=["rows=worker"]),
    S("review/needs-attention", "reviewstub.js", "recent=1&file=1&flagged=1", has=["rows=worker,db"]),
    S("review/select", "reviewstub.js", "recent=1&file=1&sel=web",
      has=["detail=web", "trace_series", "trace_daily", "Suggested requests", "−99%", "request = limit 6.0 GiB", "range=All", "4 pods seen", "1 rollout"],
      lacks=["pod(s)"]),
    S("review/select-db", "reviewstub.js", "recent=1&file=1&sel=db",
      has=["detail=db", "Throttled 38%", "Above the memory limit", "OOM kill", "limit 1 core", "heat="]),
    S("review/quick-range", "reviewstub.js", "recent=1&file=1&sel=web&quick=6h", has=["trace_series:zoomed", "range=Last 6h"]),
    S("review/drag-zoom", "reviewstub.js", "recent=1&file=1&sel=db&zoom=1", has=["trace_series:zoomed", "range=none"]),
    S("review/heat-memory", "reviewstub.js", "recent=1&file=1&sel=web&heat=mem", has=["High (2.3 GiB)"], budget=12000),
    S("review/back", "reviewstub.js", "recent=1&file=1&back=1", has=["page=Traces", "Recent"]),
    # The overview: every container added up, following the namespace filter.
    S("review/overview", "reviewstub.js", "recent=1&file=1",
      has=["trace_overview:all", "Overview of the whole trace", "Used", "Busiest minute", "Requested"], lacks=["Average pod"]),
    S("review/overview-dark", "reviewstub.js", "recent=1&file=1", scheme=DARK, has=["Overview of the whole trace"]),
    S("review/overview-namespace", "reviewstub.js", "recent=1&file=1&ns=shop", has=["trace_overview:shop", "Overview of shop"]),
    S("review/overview-zoom", "reviewstub.js", "recent=1&file=1&ovzoom=1", has=["trace_overview:all:zoomed", "Zoom out"], lacks=["trace_series"]),
    S("review/overview-idle", "reviewstub.js", "recent=1&file=1&ovtip=1", has=["idle", "% of requested", "containers"]),
    S("review/overview-hide", "reviewstub.js", "recent=1&file=1&ovhide=1", has=["Show"], lacks=["Busiest minute"]),
    S("review/overview-and-row", "reviewstub.js", "recent=1&file=1&sel=web", has=["Busiest minute", "Busiest pod", "Average pod"]),
    # ---- Applying suggestions (reviewstub.js) ----
    S("apply/one-row", "reviewstub.js", "recent=1&file=1&sel=web&apply=1",
      has=["dialogs=Apply suggested requests", "rightsize_preview:web@acme-uat", "1500m → 15m", "6Gi → 2944Mi", "Dry run accepted", "Managed by Helm release shop-web", "applybtn=on"],
      lacks=["rightsize_apply"]),
    S("apply/one-row-go", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&go=1", has=["rightsize_apply:web@acme-uat", "✓ Applied", "Done"]),
    S("apply/copy-snippet", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&copy=1", has=["copied=yes", "Copied"]),
    S("apply/selected", "reviewstub.js", "recent=1&file=1&check=web,worker,db&applysel=1&refuse=1",
      has=["Apply selected (3)…", "rightsize_preview:db@acme-uat", "CPU limit raised to match: 1 → 1350m", "memory limit raised to match: 1000Mi → 1104Mi",
           "The server refused the dry run: exceeded quota", "Apply to 2 workloads"]),
    S("apply/selected-go", "reviewstub.js", "recent=1&file=1&check=web,worker,db&applysel=1&refuse=1&go=1",
      has=["rightsize_apply:web@acme-uat", "rightsize_apply:worker@acme-uat"], lacks=["rightsize_apply:db"]),
    S("apply/failure-shown", "reviewstub.js", "recent=1&file=1&check=web,worker&applysel=1&go=1&fail=1",
      has=["✓ Applied", "✗ admission webhook \"policy.acme.io\" denied the request"]),
    S("apply/cron-and-job-rows", "reviewstub.js", "recent=1&file=1&sel=nightly", has=["No suggestion"], lacks=["Apply…"]),
    S("apply/prod", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&ctx=prod&go=1",
      has=["acme-prd looks like production", "rightsize_apply:web@acme-prd"], lacks=["Type its name"]),
    S("apply/locked", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&locked=1&go=1",
      has=["acme-uat is read-only", "Allow changes…", "applybtn=off"], lacks=["rightsize_apply"]),
    S("apply/unlock-then-apply", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&locked=1&unlock=1&go=1", has=["rightsize_apply:web@acme-uat"]),
    S("apply/other-context", "reviewstub.js", "recent=1&file=1&sel=web&other=1", has=["Connect to acme-stg to apply suggestions."], lacks=["Apply…"]),
    S("apply/dark", "reviewstub.js", "recent=1&file=1&check=web,worker&applysel=1", scheme=DARK, has=["Apply to 2 workloads"]),
    # ---- Notes, compare, export (reviewstub.js) ----
    S("notes/shown", "reviewstub.js", "recent=1&file=1&noted=1&sel=web",
      has=["Note on this trace", "Peak season week; compare after the resize.", "Note on this workload", "Agreed with the team: 250m"]),
    S("notes/add-trace-note", "reviewstub.js", "recent=1&file=1&note=Before the resize",
      has=["note:trace=Before the resize", "Note on this trace Before the resize"]),
    S("notes/add-workload-note", "reviewstub.js", "recent=1&file=1&sel=worker&wnote=Raise to 1.4 cores", has=["note:worker=Raise to 1.4 cores"]),
    S("notes/leaving-asks", "reviewstub.js", "recent=1&file=1&note=Half written&leave=1",
      has=["Discard unsaved changes?", "the note on this trace"], lacks=["note:trace"]),
    S("compare/from-recent", "reviewstub.js", "recent=1&page=1&compare=1",
      has=["compare:2", "Comparing 2 traces", "cmprows=web,db,nightly", "Request 1.5 cores → 15m (−99%)", "Only in trace 1", "OOM ×2"]),
    S("compare/changed-only", "reviewstub.js", "recent=1&page=1&compare=1&changed=1", has=["cmprows=web", "1 of 3 rows"]),
    S("compare/with-another", "reviewstub.js", "recent=1&file=1&cmpwith=1", has=["choose_trace_to_open", "compare:2", "Comparing 2 traces"]),
    S("compare/dark", "reviewstub.js", "recent=1&page=1&compare=1", scheme=DARK, has=["Comparing 2 traces"]),
    # A row's charts from each trace, overlaid; lined up since start unless asked.
    S("compare/overlay", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=web",
      has=["overlay:web@before", "overlay:web@after", "Line up by", "Trace 1 · acme-uat-shop-before.trace", "Trace 2 · acme-uat-shop-after.trace", "+1d",
           "heads=var(--trace-1)|var(--trace-2)"],
      lacks=["Showing traces"]),
    S("compare/overlay-week", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=web&cmpalign=week", has=["Line up by"], lacks=["+1d"]),
    S("compare/overlay-tip", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=web&cmptip=1", has=["Trace 1:", "Trace 2:", "request 1.5 cores", "request 15m"]),
    S("compare/overlay-one-trace", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=nightly", has=["overlay:nightly@before"], lacks=["overlay:nightly@after"]),
    S("compare/overlay-close", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=web&cmpclose=1", has=["select one to overlay"], lacks=["Line up by"]),
    S("compare/overlay-dark", "reviewstub.js", "recent=1&page=1&compare=1&cmpsel=web", scheme=DARK, has=["Line up by"]),
    S("export/saved", "reviewstub.js", "recent=1&file=1&export=1", has=["Report saved:", "acme-uat-shop-report.html"]),
    S("export/failed", "reviewstub.js", "recent=1&file=1&export=1&exportfail=1", has=["The report wasn't saved: permission denied"]),
    S("trace/switch-without-trace", "tracestub.js", "switch=1", has=["connects=2", "dialogs=none"]),
    # ---- Settings / appearance ----
    # ---- Giving up on a failing connection (connstub.js) ----
    S("conn/startup-fail", "connstub.js", "start=fail",
      has=["Not connected to acme-uat", "token has expired", "connect=1", "connect-button=true"]),
    # The Pods clicks come before the sidebar exists, so this lands on the Cluster page.
    S("conn/startup-connect", "connstub.js", "start=fail&click=connect",
      has=["connect=2", "connect-button=false", "overview=1"], lacks=["Not connected"]),
    S("conn/watch-lost", "connstub.js", "watch=stop&check=fail",
      has=["Not connected to acme-uat", "check=1", "watch=1", "connect-button=true"], lacks=["api-1"]),
    S("conn/watch-lost-connect", "connstub.js", "watch=stop&check=fail&click=connect",
      has=["connect=2", "watch=2", "connect-button=false", "api-1"]),
    S("conn/watch-stopped-retry", "connstub.js", "watch=stop&check=ok",
      has=["Stopped after repeated failures", "check=1", "retry-button=true", "connect-button=false"]),
    S("conn/watch-retry", "connstub.js", "watch=stop&check=ok&click=retry", has=["watch=2", "retry-button=false"]),
    S("conn/overview-gives-up", "connstub.js", "overview=fail&check=late", budget=100000,
      has=["overview=3", "connect-button=true"]),
    S("conn/overview-stops-polling", "connstub.js", "overview=fail&check=ok", budget=100000,
      has=["overview=3", "check=1", "Stopped refreshing after repeated failures", "connect-button=false"]),
    S("conn/overview-refresh-restarts", "connstub.js", "overview=fail&check=ok&click=refresh", budget=100000,
      has=["overview=4"]),
    # A dropped VPN: nothing errors, the watch just goes quiet; the background check notices.
    S("conn/vpn-drop", "connstub.js", "vpn=drop&at=33000", budget=40000,
      has=["Not connected to acme-uat", "check your network or VPN", "check=3", "watch=1", "connect-button=true"]),
    S("conn/vpn-not-yet", "connstub.js", "vpn=drop&at=22000", budget=30000,
      has=["check=2", "connect-button=false", "api-1"]),
    S("conn/vpn-reconnect", "connstub.js", "vpn=drop&click=connect&at=33000", budget=40000,
      has=["connect=2", "watch=2", "connect-button=false", "api-1"]),
    # A Cluster page refresh lost with the VPN must not block refreshing after Connect.
    S("conn/vpn-hung-refresh", "connstub.js", "overview=hang&vpn=drop&click=connect&at=33000", budget=40000,
      has=["connect=2", "overview=2", "connect-button=false"]),
    S("settings/auto-follows-dark", "settingsstub.js", "open=1", scheme=DARK, has=["data-theme=none", "bg=rgb(22, 25, 30)", "Auto*"]),
    S("settings/light-over-dark", "settingsstub.js", "pick=light", scheme=DARK, has=["data-theme=light", "bg=rgb(246, 247, 249)", "native=auto,light"]),
    S("settings/dark-over-light", "settingsstub.js", "pick=dark", has=["data-theme=dark", "bg=rgb(22, 25, 30)"]),
    S("settings/saved-dark-restart", "settingsstub.js", "theme=dark", has=["data-theme=dark", "native=dark"]),
    S("settings/back-to-auto", "settingsstub.js", "theme=light&pick=auto", scheme=DARK, has=["data-theme=none", "saved=auto"]),
    S("settings/outside-click-closes", "settingsstub.js", "open=1&click=outside", has=["dialog=closed"]),
    S("settings/inside-click-stays", "settingsstub.js", "open=1&click=inside", has=["dialog=open"]),
    S("settings/no-auto", "settingsstub.js", "open=1&noauto=1", has=["buttons=Light*|Dark"], lacks=["Auto"]),
    # ---- File › Manage Clusters… (clusterstub.js) ----
    S("manage/lists", "clusterstub.js", "manage=1",
      has=["Manage Clusters", "rows=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps", "menu=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps", "kubectl current", "this window",
           "prod=shop-prod ", "locked-rows=acme-uat ", "https://ABC.gr7.eu-west-1.eks.amazonaws.com", "managed_contexts", "dialog=closed"], lacks=["several files", "cloud_accounts"]),
    S("manage/up", "clusterstub.js", "manage=1&up=shop-prod",
      has=['reorder_contexts {"order":["acme-uat","shop-prod","gke_acme_us-central1_web","aks-apps"]}', "rows=acme-uat|shop-prod|gke_acme_us-central1_web|aks-apps", "menu=acme-uat|shop-prod|gke_acme_us-central1_web|aks-apps"]),
    S("manage/down", "clusterstub.js", "manage=1&down=acme-uat", has=['"order":["gke_acme_us-central1_web","acme-uat","shop-prod","aks-apps"]', "menu=gke_acme_us-central1_web|acme-uat|"]),
    S("manage/first-stays", "clusterstub.js", "manage=1&up=acme-uat", has=["nothing sent", "rows=acme-uat|gke_acme_us-central1_web|"]),
    S("manage/other-file", "clusterstub.js", "manage=1&files=2&up=aks-apps",
      has=["nothing sent", "several files", "/Users/me/.kube/aks", "rows=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps"]),
    S("manage/drag-before", "clusterstub.js", "manage=1&drag=shop-prod&onto=acme-uat", has=['"order":["shop-prod","acme-uat","gke_acme_us-central1_web","aks-apps"]']),
    S("manage/drag-after", "clusterstub.js", "manage=1&drag=acme-uat&onto=shop-prod&after=1", has=['"order":["gke_acme_us-central1_web","shop-prod","acme-uat","aks-apps"]']),
    S("manage/drag-across-files", "clusterstub.js", "manage=1&files=2&drag=aks-apps&onto=acme-uat", has=["nothing sent", "rows=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps"]),
    S("manage/reorder-fails", "clusterstub.js", "manage=1&up=shop-prod&badorder=1", has=["another program", "rows=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps"]),
    S("manage/remove-asks", "clusterstub.js", "manage=1&remove=gke_acme_us-central1_web",
      has=['Remove "gke_acme_us-central1_web" from the kubeconfig?', 'Its cluster "gke_acme_us-central1_web" and user "gke_acme_us-central1_web" go too', "Keep"], lacks=["removed=g"]),
    S("manage/remove-shared", "clusterstub.js", "manage=1&remove=shop-prod", has=['Its user "shared" goes too: nothing else uses it.']),
    S("manage/remove", "clusterstub.js", "manage=1&remove=gke_acme_us-central1_web&yes=1",
      has=["removed=gke_acme_us-central1_web", 'Removed "gke_acme_us-central1_web", with its cluster', "rows=acme-uat|shop-prod|aks-apps", "menu=acme-uat|shop-prod|aks-apps", "manage=open"]),
    S("manage/remove-this-window", "clusterstub.js", "manage=1&here=shop-prod&remove=shop-prod",
      has=["re:this window", "connected=shop-prod"], lacks=["from the kubeconfig?"]),
    S("manage/remove-refused", "clusterstub.js", "manage=1&remove=aks-apps&yes=1&inuse=1",
      has=["A window is connected to", "rows=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps"], lacks=["removed=a"]),
    S("manage/mark", "clusterstub.js", "manage=1&mark=acme-uat",
      has=['set_production {"context":"acme-uat","production":true}', "prod=acme-uat|shop-prod ", "prod-icons=acme-uat|shop-prod "]),
    S("manage/unmark", "clusterstub.js", "manage=1&mark=shop-prod", has=['"production":false', "prod= ", "prod-icons= "]),
    S("manage/other-window", "clusterstub.js", "manage=1&elsewhere=gke_acme_us-central1_web",
      has=["another window", "locked-rows=acme-uat|gke_acme_us-central1_web ", "calls=contexts_in_use_elsewhere,"]),
    S("manage/other-window-later", "clusterstub.js", "manage=1&remove=aks-apps&later=aks-apps", has=["locked-rows=acme-uat|aks-apps ", "confirm=disabled"]),
    S("manage/other-window-left", "clusterstub.js", "manage=1&elsewhere=aks-apps&later=gke_acme_us-central1_web", has=["locked-rows=acme-uat|gke_acme_us-central1_web "]),
    S("manage/dark", "clusterstub.js", "manage=1&files=2", scheme=DARK, has=["Manage Clusters"]),
    # The connected context's cloud, in the Context menu and on the Cluster page.
    S("manage/logo-kubernetes", "clusterstub.js", "manage=1", has=["menu-logo=kubernetes", "page-logo=kubernetes"]),
    S("manage/logo-aws", "clusterstub.js", "manage=1&here=shop-prod", has=["menu-logo=aws", "page-logo=aws", "connected=shop-prod"]),
    S("manage/logo-azure", "clusterstub.js", "manage=1&here=aks-apps", has=["menu-logo=azure", "page-logo=azure"]),
    # ---- The kubeconfig changed outside the app (kubeconfig_watch.rs, via clusterstub.js) ----
    S("outside/context-removed", "clusterstub.js", "manage=1&closed=1&gone=1",
      has=["connected=shop-prod", "shown=shop-prod", "notice=“acme-uat” was removed from your kubeconfig, so this window moved to “shop-prod”."],
      lacks=["menu=acme-uat"]),
    S("outside/no-contexts", "clusterstub.js", "manage=1&closed=1&empty=1",
      has=["No clusters", "File › Manage Clusters…", "Add Cluster", "dialog=open", "no contexts left", "shown=No contexts", "menu= "]),
    S("outside/starts-empty", "clusterstub.js", "startempty=1&closed=1",
      has=["No clusters", "dialog=open", "connected=acme-uat"], lacks=[",connect,"]),
    S("outside/comes-back", "clusterstub.js", "closed=1&empty=1&readd=1", has=["connected=lab", "shown=lab", "menu=lab "], lacks=["No clusters"]),
    # ---- The Context menu (context_picker.rs, via clusterstub.js) ----
    S("context-menu/opens", "clusterstub.js", "manage=1&closed=1&menu=1",
      has=["expanded=true", "shown=acme-uat", "prod-icons=shop-prod ", "active=acme-uat", "menu=acme-uat|gke_acme_us-central1_web|shop-prod|aks-apps"], lacks=["⚠"]),
    S("context-menu/click", "clusterstub.js", "manage=1&closed=1&ctx=aks-apps", has=["connected=aks-apps", "expanded=false", "shown=aks-apps", "menu-logo=azure"]),
    S("context-menu/same-does-nothing", "clusterstub.js", "manage=1&closed=1&ctx=acme-uat", has=["expanded=false", "calls=contexts_in_use_elsewhere,trace_status,list_contexts,connect,cluster_overview "]),
    S("context-menu/arrows", "clusterstub.js", "manage=1&closed=1&keys=ArrowDown,ArrowDown,ArrowDown,Enter", has=["connected=shop-prod", "expanded=false"]),
    S("context-menu/end-and-up", "clusterstub.js", "manage=1&closed=1&keys=ArrowDown,End,ArrowUp", has=["expanded=true", "active=shop-prod", "connected=acme-uat"]),
    S("context-menu/letter", "clusterstub.js", "manage=1&closed=1&keys=ArrowDown,a,Enter", has=["connected=aks-apps"]),
    S("context-menu/esc", "clusterstub.js", "manage=1&closed=1&menu=1&keys=Escape", has=["expanded=false", "connected=acme-uat"]),
    S("context-menu/outside", "clusterstub.js", "manage=1&closed=1&menu=1&outside=1", has=["expanded=false", "connected=acme-uat"]),
    # ---- Manage Clusters › Add Cluster… (clusterstub.js) ----
    S("cluster/opens-on-aws", "clusterstub.js", "",
      has=["Add Cluster", "re:AWS.*Google Cloud.*Azure.*Kubernetes", "Saved to /Users/me/.kube/config", "Default credentials", "re:api\\s*eu-west-1\\s*1\\.33\\s*ACTIVE", "list:@us-east-2", "dialog=open", "add=enabled"]),
    S("cluster/kubernetes-lists-nothing", "clusterstub.js", "src=k8s", has=["API server", "add=disabled"], lacks=["Default credentials"]),
    S("cluster/manual-token", "clusterstub.js", "src=k8s&name=lab&server=https://10.0.0.1:6443&token=abc&ns=shop&go=1",
      has=['add_cluster {"cluster":{"name":"lab","server":"https://10.0.0.1:6443","certificate_authority":"","insecure_skip_tls_verify":false,'
           '"proxy_url":"","auth":{"kind":"token","token":"abc"},"namespace":"shop","make_current":false}}', "dialog=closed", "connected=lab", "contexts=2", "manage=open", "rows=acme-uat|lab", "menu=acme-uat|lab"]),
    S("cluster/manual-exec-current", "clusterstub.js", "src=k8s&auth=exec&name=lab&server=https://k.example&cmd=kubelogin&args=get-token|--login=azurecli&current=1&go=1",
      has=['"auth":{"kind":"exec","command":"kubelogin","args":["get-token","--login=azurecli"]}', '"make_current":true']),
    S("cluster/manual-no-connect", "clusterstub.js", "src=k8s&name=lab&server=https://10.0.0.1:6443&auth=none&noconnect=1&go=1",
      has=['"auth":{"kind":"none"}', "dialog=closed", "connected=acme-uat", "contexts=2"]),
    S("cluster/manual-bad-server", "clusterstub.js", "src=k8s&name=lab&server=10.0.0.1&token=abc&go=1",
      has=["nothing sent", "starts with https://", "add=disabled"]),
    S("cluster/manual-ca-and-insecure", "clusterstub.js", "src=k8s&name=lab&server=https://h&token=t&ca=/ca.crt&insecure=1&go=1",
      has=["nothing sent", "not both"]),
    S("cluster/manual-refused", "clusterstub.js", "src=k8s&name=acme-uat&server=https://h&token=t&go=1&fail=1",
      has=["already exists", "dialog=open", "add=enabled", "connected=acme-uat"]),
    S("cluster/gke-lists-default-project", "clusterstub.js", "src=gke",
      has=["Project", "Acme Dev (acme-dev)", "list:acme-ops", "web", "us-central1-a", "1.33.1-gke.100", "gke-gcloud-auth-plugin", "add=disabled"]),
    S("cluster/gke-add", "clusterstub.js", "src=gke&pick=batch&go=1",
      has=['add_cloud_cluster {"provider":"gke","account":"acme-ops","cluster":{"name":"batch","location":"us-central1-a","group":"",'
           '"version":"1.32.9-gke.200","status":"RUNNING","server":"34.9.9.9","entry":"gke_acme-ops_us-central1-a_batch"},"alias":"","make_current":false}', "connected=gke_acme-ops_us-central1-a_batch"]),
    S("cluster/eks-default-credentials", "clusterstub.js", "src=eks&alias=api&go=1",
      has=["list:@us-east-2", 'add_cloud_cluster {"provider":"eks","account":"","cluster":{"name":"api"', '"alias":"api"', "connected=api"]),
    S("cluster/eks-profile-region", "clusterstub.js", "src=eks&profile=work", has=["list:work@eu-west-1"]),
    S("cluster/eks-needs-region", "clusterstub.js", "src=eks&noregion=1", has=["Default credentials", "Choose a region.", "add=disabled"], lacks=["list:"]),
    S("cluster/eks-typed-region", "clusterstub.js", "src=eks&noregion=1&region=ap-south-1", has=["list:@ap-south-1"]),
    S("cluster/aks-default-subscription", "clusterstub.js", "src=aks",
      has=["Subscription", "list:2222", "westeurope · rg-apps", "Region · group", "kubelogin", "add=enabled"]),
    # Already in the kubeconfig: by the entry the CLI writes, or by the API server.
    S("cluster/gke-already", "clusterstub.js", "src=gke&known=1&pick=web",
      has=["added=web ", "This cluster is already in your kubeconfig as contexts “web” and “web-admin”.", "credentials have changed"]),
    S("cluster/gke-other-not-already", "clusterstub.js", "src=gke&known=1&pick=batch", has=["added=web "], lacks=["already in your kubeconfig"]),
    S("cluster/aks-already", "clusterstub.js", "src=aks&known=1", has=["added=aks-apps ", "as context “aks-apps-ctx”."]),
    S("cluster/eks-not-already", "clusterstub.js", "src=eks&known=1", has=["added= "], lacks=["already in your kubeconfig"]),
    S("cluster/manual-same-server", "clusterstub.js", "src=k8s&known=1&server=https://34.1.2.3:443/",
      has=["This server is already in your kubeconfig as “web-admin”."]),
    S("cluster/manual-new-server", "clusterstub.js", "src=k8s&known=1&server=https://10.9.9.9", lacks=["already in your kubeconfig"]),
    S("cluster/cli-missing", "clusterstub.js", "src=aks&nocli=1", has=["The az command isn't installed", "Try again", "add=disabled"]),
    S("cluster/no-clusters", "clusterstub.js", "src=gke&noclusters=1", has=["No clusters here.", "add=disabled"]),
    S("cluster/dark", "clusterstub.js", "src=gke", scheme=DARK, has=["Google Cloud"]),
    # ---- YAML editing ----
    S("edit/button-opens-editor", "configstub.js", "kind=cm&yamledit=1",
      has=['open_editor {"target":{"kind":{"group":"","version":"v1","kind":"ConfigMap"', '"name":"acme-api-config"']),
    S("edit/no-button-for-helm", "helmstub.js", "", lacks=["open_editor"]),
    S("edit/prod-chip-icon", "editorstub.js", "edit=edit-1&prod=1", has=["prod-chip=acme-prd+icon"], lacks=["⚠"]),
    S("edit/loads-highlighted", "editorstub.js", "edit=edit-1", has=["calls=start", "highlighted=true", "No changes", "replicas"]),
    S("edit/change-marks-dirty", "editorstub.js", "edit=edit-1&change=1", has=["dirty=true", "Edited · not saved"]),
    S("edit/no-changes-no-save", "editorstub.js", "edit=edit-1&save=1", has=["No changes to save."], lacks=["check"]),
    # Ace checks syntax in a web worker, which needs real time.
    S("edit/syntax-error-marked", "editorstub.js", "edit=edit-1&bad=1", has=["re: errors=[1-9]"], budget=6000, real_time=True),
    S("edit/syntax-error-blocks-save", "editorstub.js", "edit=edit-1&bad=1&save=1",
      has=["Fix the YAML first: line 4", "re: errors=[1-9]"], lacks=["check"], budget=6000, real_time=True),
    S("edit/save-unlocked", "editorstub.js", "edit=edit-1&change=1&save=1&confirm=1",
      has=["calls=start,check,save,close", "dirty=false"]),
    S("edit/save-shows-diff", "editorstub.js", "edit=edit-1&change=1&save=1",
      has=["modal=Save Deployment?", "1 line added, 1 removed.", "replicas: 3"], lacks=[",save"]),
    S("edit/locked-asks-to-allow", "editorstub.js", "edit=edit-1&change=1&save=1&locked=1",
      has=["modal=Allow changes?"], lacks=[",save"]),
    S("edit/locked-then-saves", "editorstub.js", "edit=edit-1&change=1&save=1&locked=1&confirm=1",
      has=["unlock:acme-uat", ",save,close"]),
    S("edit/prod-unlock-needs-name", "editorstub.js", "edit=edit-1&prod=1&locked=1&change=1&save=1&confirm=1", lacks=[",save", "unlock:"]),
    S("edit/prod-typed-saves", "editorstub.js", "edit=edit-1&prod=1&locked=1&change=1&save=1&typed=1&confirm=1",
      has=["unlock:acme-prd", ",save,close"]),
    S("edit/server-rejects", "editorstub.js", "edit=edit-1&change=1&save=1&dryfail=1",
      has=["must be greater than or equal to 0", "modal=none"], lacks=[",save"]),
    S("edit/cancel-clean-closes", "editorstub.js", "edit=edit-1&cancel=1", has=["calls=start,close"]),
    S("edit/cancel-dirty-asks", "editorstub.js", "edit=edit-1&change=1&cancel=1", has=["modal=Discard your edits?"], lacks=[",close"]),
    S("edit/cancel-dirty-keep", "editorstub.js", "edit=edit-1&change=1&cancel=1&keep=1", has=["modal=none", "dirty=true"], lacks=[",close"]),
    S("edit/cancel-dirty-discard", "editorstub.js", "edit=edit-1&change=1&cancel=1&discard=1", has=["calls=start,close", "dirty=false"]),
    S("edit/window-close-asks", "editorstub.js", "edit=edit-1&change=1&closereq=1", has=["modal=Discard your edits?"], lacks=[",close"]),
    S("edit/window-close-clean", "editorstub.js", "edit=edit-1&closereq=1", has=["calls=start,close"]),
    S("edit/reload-dirty-asks", "editorstub.js", "edit=edit-1&change=1&reload=1&discard=1", has=["calls=start,start", "dirty=false"]),
    # ---- Find in a detail panel (find.js) ----
    S("find/configmap", "configstub.js", "kind=cm&find=8080",
      has=["find count=1 of 1", "current=8080", "painted=1", "bar=true"]),
    S("find/button", "configstub.js", "kind=cm&find=helm&findbtn=1", has=["find count=1 of", "bar=true", "re:painted=[1-9]"]),
    S("find/none", "configstub.js", "kind=cm&find=no-such-text", has=["count=No matches", "painted=0"]),
    S("find/next-wraps", "configstub.js", "kind=cm&find=acme-api&next=1", has=["re:count=2 of [2-9]"]),
    S("find/prev-wraps", "configstub.js", "kind=cm&find=acme-api&prev=1", has=["re:count=([2-9]) of \\1"]),
    S("find/esc-closes-bar-only", "configstub.js", "kind=cm&find=info&findesc=1", has=["bar=false", "panel=true", "painted=0"]),
    S("find/skips-revealed-secret", "configstub.js", "kind=secret&show=1&find=8f2a9c", has=["count=No matches", "tok_8f2a9c1e77"]),
    S("find/secret-keys-still-found", "configstub.js", "kind=secret&find=api_token", has=["count=1 of 1", "current=API_TOKEN"]),
    S("find/configmap-yaml", "configstub.js", "kind=cm&findtab=yaml&find=kind:", has=["count=1 of 1"]),
    S("find/secret-yaml-skipped", "configstub.js", "kind=secret&findtab=yaml&find=kind:", has=["count=No matches"]),
    S("find/opens-collapsed-section", "workloadstub.js", "collapse=pods&find=m8n3p",
      has=["re:count=1 of [0-9]", "current=m8n3p", "section=Pods", "closed=0"]),
    S("find/collapsed-not-opened-without-match", "workloadstub.js", "collapse=pods&find=no-such-pod", has=["count=No matches", "closed=1"]),
    # ---- Esc (esc.js) ----
    # The detail panel: closes, except while typing; unsaved edits ask first.
    S("esc/panel-closes", "configstub.js", "kind=cm&esc=2800", has=["before: dialog=none detail=yes after: dialog=none detail=no"]),
    S("esc/panel-field-stays", "configstub.js", "kind=cm&esc=2800&escfrom=field", has=["from=field", "after: dialog=none detail=yes"]),
    S("esc/panel-select-closes", "configstub.js", "kind=cm&esc=2800&escfrom=select", has=["from=select", "after: dialog=none detail=no"]),
    S("esc/panel-unsaved-asks", "configstub.js", "kind=cm&edit=1&change=1&esc=2800",
      has=["after: dialog=Discard unsaved changes? detail=yes"]),
    # Modal's rules, once each (every dialog is a Modal, and must give it on_cancel):
    # cancels from a text field, only the dialog, not on a held key's repeats, not while busy.
    S("esc/delete", "stub.js", "pod=limited&ctx=prod&del=1&esc=dialog",
      has=["before: dialog=Delete Pod?", "after: dialog=none detail=yes"]),
    S("esc/held-closes-dialog-only", "stub.js", "pod=limited&del=1&esc=dialog&eschold=1",
      has=["before: dialog=Delete Pod?", "after: dialog=none detail=yes"]),
    S("esc/busy-delete", "stub.js", "pod=limited&del=2&hang=delete_resource&esc=busy",
      has=["before: dialog=Delete Pod?", "after: dialog=Delete Pod? detail=yes"]),
    S("esc/busy-editor-save", "editorstub.js", "edit=edit-1&change=1&save=1&confirm=1&hang=save&esc=busy",
      has=["before: dialog=Save Deployment?", "modal=Save Deployment?", "calls=start,check,save"], lacks=[",close"]),
    # Dialogs whose cancel is more than closing.
    S("esc/unsaved-prompt-keeps", "configstub.js", "kind=cm&edit=1&change=1&leave=row&esc=3300",
      has=["before: dialog=Discard unsaved changes?", "after: dialog=none detail=yes", "prompt=false", "editing=true", "unsaved=true"]),
    S("esc/unlock", "configstub.js", "kind=cm&ctx=prod&locked=1&edit=1&change=1&save=1&esc=3400&escfrom=field",
      has=["from=field", "before: dialog=Allow changes?", "after: dialog=none detail=yes"], lacks=["save_config_data"]),
    S("esc/upload-replace-back", "stub.js", "pod=limited&upload=send&exists=1&esc=4200",
      has=["before: dialog=Upload file", "after: dialog=Upload file"], lacks=["overwrite=true"]),
    S("esc/busy-upload", "stub.js", "pod=limited&upload=send&hang=upload_file&esc=busy",
      has=["before: dialog=Upload file", "after: dialog=Upload file detail=yes"]),
    S("esc/busy-editor-unlock", "editorstub.js", "edit=edit-1&change=1&save=1&locked=1&confirm=1&hang=unlock&esc=busy",
      has=["before: dialog=Allow changes?", "modal=Allow changes?"], lacks=["modal=Save"]),
    # A late result from a dialog closed and reopened is dropped.
    S("esc/download-late-dropped", "stub.js", "pod=limited&download=done&slow=download_file&esc=3800&again=4400",
      has=["after: dialog=none", "Download file"], lacks=["/Users/me/Downloads/app.log"]),
    S("esc/forward-late-ports-dropped", "fwd.js", "page=dialog&late=1&esc=2200",
      has=["before: dialog=Port forward", "grafana-web"], lacks=["http-web"]),
    S("esc/forward-loading", "fwd.js", "page=dialog&hang=forward_ports&esc=dialog",
      has=["before: dialog=Port forward", "after: dialog=none detail=yes"]),
    # Traces' dialogs: Esc does what Keep tracing / Close / Cancel does, never stops or applies.
    S("esc/trace-start-keeps-panel", "tracestub.js", "this=1&esc=1", has=["dialogs=none", "detail=yes"], lacks=["choose_trace_file"]),
    S("esc/trace-panel", "tracestub.js", "running=1&panel=1&esc=1", has=["dialogs=none", "indicator=Trace"], lacks=["stop_trace"]),
    S("esc/trace-stopping", "tracestub.js", "running=1&stop=1&hang=1&esc=1", has=["stop_trace", "dialogs=Trace running", "Stopping…"]),
    S("esc/trace-quit-keeps", "tracestub.js", "running=1&quit=1&esc=1", has=["dialogs=none", "indicator=Trace"], lacks=["close_discarding"]),
    S("esc/trace-quit-busy", "tracestub.js", "running=1&quit=1&confirm=1&hang=1&esc=1", has=["close_discarding", "dialogs=A trace is running"]),
    S("esc/trace-switch-keeps", "tracestub.js", "running=1&switch=1&esc=1", has=["connects=1", "dialogs=none", "indicator=Trace"], lacks=["stop_trace"]),
    S("esc/apply", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&esc=dialog",
      has=["before: dialog=Apply suggested requests", "after: dialog=none", "detail=web"], lacks=["rightsize_apply"]),
    S("esc/apply-busy", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&go=1&hang=1&esc=busy",
      has=["rightsize_apply", "after: dialog=Apply suggested requests", "Applying…"]),
    S("esc/apply-unlock-on-top", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&locked=1&unlockesc=1",
      has=["dialogs=Apply suggested requests"], lacks=["rightsize_apply", "dialogs=Apply suggested requests|"]),
    # ---- Logs and shell windows ----
    S("logs/window", "logstub.js", "logs=app&merged=1", has=["GET /healthz", "web-7d9f-abcde/app", "Wrap"], lacks=["bad response"]),
    S("logs/target-filter", "logstub.js", "logs=app&target=1", has=["Reconciling ScaledObject"], lacks=["GET /healthz"]),
    S("exec/window", "execstub.js", "exec=app", has=["Shell"]),
    S("about/window", "aboutstub.js", "about",
      has=["Version 0.1.0", "Desktop app for browsing and controlling Kubernetes clusters.",
           "An it-problems application. Copyright © 2026 it-problems. Released under the MIT License.",
           "Source code", "Third-party licenses", "Built on", "Tauri", "Leptos", "xterm.js", "and 3 open-source Rust crates",
           "Third-party software (4)"],
      lacks=["Permission is hereby granted"]),
    S("about/license", "aboutstub.js", "about&tab=license", has=["Permission is hereby granted", "Copyright (c) 2026 it-problems"],
      lacks=["Built on"]),
    S("about/third-party", "aboutstub.js", "about&tab=third-party&open=1",
      has=["MIT License · serde, tokio", "Apache License 2.0 · k8s-openapi", "Vendored JavaScript: xterm", "tokio 1.48.0",
           "3 open-source Rust crates and 1 JavaScript libraries", "All 4", "MIT License 2", "JavaScript 1"]),
    S("about/filter", "aboutstub.js", "about&tab=third-party&filter=openapi", has=["Apache License 2.0 · k8s-openapi"], lacks=["MIT License · serde", "Vendored JavaScript"]),
    S("about/chip", "aboutstub.js", "about&tab=third-party&chip=JavaScript", has=["Vendored JavaScript: xterm"], lacks=["MIT License · serde", "k8s-openapi"]),
    S("about/source-link", "aboutstub.js", "about&link=Source", has=["sent=open_url:https://github.com/cjcooper/k8s-subduer"]),
    S("about/licenses-link", "aboutstub.js", "about&link=Third-party", has=["MIT License · serde, tokio"], lacks=["Built on"]),
    S("about/esc", "aboutstub.js", "about&esc=1", has=["sent=close_about"]),
    S("about/icon-light", "aboutstub.js", "about", has=["icon=light "], lacks=["icon=light,"]),
    S("about/icon-dark", "aboutstub.js", "about", scheme=DARK, has=["icon=dark "], lacks=["icon=dark,"]),
    S("about/icon-light-over-dark", "aboutstub.js", "about&theme=light", scheme=DARK, has=["icon=light "], lacks=["icon=light,"]),
]


def build(out):
    subprocess.run(["trunk", "build", "--dist", out], cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    index = open(os.path.join(out, "index.html")).read()
    for f in os.listdir(HARNESS):
        if f.endswith(".js"):
            shutil.copy(os.path.join(HARNESS, f), out)
            # esc.js and find.js ride along with every stub (they do nothing without `esc=` / `find=`); no pages of their own.
            if f in ("esc.js", "find.js"):
                continue
            page =index.replace("<head>", f'<head><script src="{f}"></script><script src="esc.js"></script><script src="find.js"></script>', 1)
            open(os.path.join(out, f"page-{f[:-3]}.html"), "w").write(page)


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def serve(directory):
    handler = functools.partial(QuietHandler, directory=directory)
    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run_real_time(url, scenario):
    """Loads the page and waits `budget` ms of real time, then returns its DOM.
    For web workers (Ace's syntax check), which don't run under virtual time.
    Talks to Chrome over --remote-debugging-pipe: fd 3 in, fd 4 out, messages
    are JSON ended by a NUL byte."""
    to_chrome_r, to_chrome_w = os.pipe()
    from_chrome_r, from_chrome_w = os.pipe()
    profile = tempfile.mkdtemp(prefix="subduer-e2e-profile-")

    def child_fds():
        os.dup2(to_chrome_r, 3)
        os.dup2(from_chrome_w, 4)

    chrome = subprocess.Popen(
        [CHROME, "--headless=new", "--disable-gpu", f"--blink-settings=preferredColorScheme={scenario['scheme']}",
         "--remote-debugging-pipe", f"--user-data-dir={profile}", url],
        # 3 and 4 too: fds not listed are closed after child_fds runs.
        pass_fds=(3, 4, to_chrome_r, from_chrome_w), preexec_fn=child_fds,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    os.close(to_chrome_r)
    os.close(from_chrome_w)
    reader = os.fdopen(from_chrome_r, "rb")
    buffered = b""
    next_id = 0

    def send(method, params=None, session=None):
        nonlocal next_id, buffered
        next_id += 1
        message = {"id": next_id, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        os.write(to_chrome_w, json.dumps(message).encode() + b"\0")
        while True:
            while b"\0" not in buffered:
                chunk = reader.read1(65536)
                if not chunk:
                    raise RuntimeError("Chrome closed the debugging pipe")
                buffered += chunk
            raw, buffered = buffered.split(b"\0", 1)
            reply = json.loads(raw)
            if reply.get("id") == next_id:
                return reply.get("result", {})

    try:
        page = next(t for t in send("Target.getTargets")["targetInfos"] if t["type"] == "page")
        session = send("Target.attachToTarget", {"targetId": page["targetId"], "flatten": True})["sessionId"]
        time.sleep(scenario["budget"] / 1000)
        result = send("Runtime.evaluate", {"expression": "document.documentElement.outerHTML"}, session)
        return result.get("result", {}).get("value", "")
    finally:
        chrome.kill()
        chrome.wait()
        reader.close()
        os.close(to_chrome_w)
        shutil.rmtree(profile, ignore_errors=True)


def run(port, scenario):
    url = f"http://127.0.0.1:{port}/page-{scenario['stub'][:-3]}.html?{scenario['query']}"
    if scenario["real_time"]:
        dom = run_real_time(url, scenario)
    else:
        dom = subprocess.run(
            [CHROME, "--headless=new", "--disable-gpu", f"--blink-settings=preferredColorScheme={scenario['scheme']}",
             f"--virtual-time-budget={scenario['budget']}", "--dump-dom", url],
            capture_output=True, text=True, timeout=120,
        ).stdout
    title = html.unescape(m.group(1)) if (m := re.search(r"<title>(.*?)</title>", dom, re.S)) else ""
    body = dom.split("</head>", 1)[-1]
    body = re.sub(r"<script.*?</script>", " ", body, flags=re.S)
    text = html.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", body)))
    seen = title + "\n" + text

    def found(pattern):
        return re.search(pattern[3:], seen) is not None if pattern.startswith("re:") else pattern in seen

    failures = [f"missing {p!r}" for p in scenario["has"] if not found(p)]
    failures += [f"unexpected {p!r}" for p in scenario["lacks"] if found(p)]
    return failures, title, text


def main(args):
    show = None
    if args[:1] == ["--show"]:
        show, args = args[1], []
    picked = [s for s in SCENARIOS if (show and s["id"] == show) or (not show and (not args or any(a in s["id"] for a in args)))]
    if not picked:
        sys.exit("no scenarios match")
    out = tempfile.mkdtemp(prefix="subduer-e2e-")
    try:
        print(f"building the UI into {out}…", flush=True)
        build(out)
        server = serve(out)
        port = server.server_address[1]
        if show:
            failures, title, text = run(port, picked[0])
            print(f"title: {title}\ntext: {text[:3000]}\nfailures: {failures}")
            return
        failed = flaky = 0
        jobs = int(os.environ.get("E2E_JOBS", "1"))
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            for scenario, result in zip(picked, pool.map(lambda s: run(port, s), picked)):
                failures, title, _ = result
                if failures:
                    # Headless Chrome can run out of time on a busy machine:
                    # retry once on its own before calling it a failure.
                    failures, title, _ = run(port, scenario)
                    if not failures:
                        flaky += 1
                        print(f"ok   {scenario['id']}   (passed on retry)")
                        continue
                mark = "ok  " if not failures else "FAIL"
                print(f"{mark} {scenario['id']}")
                for f in failures:
                    print(f"       {f}   (title: {title[:200]})")
                failed += bool(failures)
        server.shutdown()
        note = f", {flaky} needed a retry" if flaky else ""
        print(f"\n{len(picked) - failed} passed, {failed} failed, {len(picked)} scenarios{note}")
        sys.exit(1 if failed else 0)
    finally:
        shutil.rmtree(out, ignore_errors=True)


if __name__ == "__main__":
    main(sys.argv[1:])
