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
    S("pod/details", "stub.js", "pod=limited", has=["Resources", "QoS class", "Burstable", "Forward", "Upload", "Shell", "Logs"]),
    S("pod/details-dark", "stub.js", "pod=unlimited", scheme=DARK, has=["Details", "YAML"]),
    # ---- Navigation ----
    # Section headings are uppercase on screen only (CSS); the text is title case.
    S("nav/sidebar", "nav.js", "open=all", has=["Workloads 7", "Config 9", "Network 5", "Storage 3", "Port Forwards", "Cluster"]),
    S("nav/forwards", "fwd.js", "page=forwards", has=["Port Forwards"]),
    # ---- Deployments / StatefulSets ----
    S("workload/details", "workloadstub.js", "rolling=1", has=["Rolling out", "Pods", "Conditions", "Resources"]),
    S("workload/restart", "workloadstub.js", "op=restart&go=1", has=["restart_workload", '"name":"acme-api"']),
    S("workload/scale", "workloadstub.js", "op=scale&to=5&go=1", has=["scale_workload", '"replicas":5']),
    S("workload/prod-needs-name", "workloadstub.js", "op=restart&ctx=prod&go=1", lacks=["restart_workload"]),
    S("workload/prod-typed", "workloadstub.js", "op=restart&ctx=prod&typed=1&go=1", has=["restart_workload", '"context":"acme-prd"']),
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
    S("config/save-prod-typed", "configstub.js", "kind=cm&ctx=prod&edit=1&change=1&save=1&typed=1&go=1",
      has=["save_config_data", '"context":"acme-prd"']),
    S("config/save-prod-untyped", "configstub.js", "kind=cm&ctx=prod&edit=1&change=1&save=1&go=1", lacks=["save_config_data"]),
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
    S("storage/reclaim-prod", "storagestub.js", "kind=pv&op=reclaim&ctx=prod&typed=1&go=1",
      has=["set_reclaim_policy", '"policy":"Retain"', '"namespace":""']),
    S("storage/reclaim-prod-untyped", "storagestub.js", "kind=pv&op=reclaim&ctx=prod&go=1", lacks=["set_reclaim_policy"]),
    S("storage/class", "storagestub.js", "kind=sc", has=["Default", "Claims", "Parameters"]),
    S("storage/attachment", "storagestub.js", "kind=va", has=["Error", "Attach error"]),
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
    S("helm/rollback-prod-untyped", "helmstub.js", "op=rollback&ctx=prod&go=1", lacks=["helm_rollback"]),
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
    S("apply/prod-needs-name", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&ctx=prod&go=1",
      has=["acme-prd looks like production", "applybtn=off"], lacks=["rightsize_apply"]),
    S("apply/prod-typed", "reviewstub.js", "recent=1&file=1&sel=web&apply=1&ctx=prod&typed=1&go=1", has=["rightsize_apply:web@acme-prd"]),
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
    # ---- YAML editing ----
    S("edit/button-opens-editor", "configstub.js", "kind=cm&yamledit=1",
      has=['open_editor {"target":{"kind":{"group":"","version":"v1","kind":"ConfigMap"', '"name":"acme-api-config"']),
    S("edit/no-button-for-helm", "helmstub.js", "", lacks=["open_editor"]),
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
    S("edit/prod-needs-names", "editorstub.js", "edit=edit-1&prod=1&locked=1&change=1&save=1&confirm=1", lacks=[",save", "unlock:"]),
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
    # ---- Esc (esc.js) ----
    # The detail panel: closes, except while typing; unsaved edits ask first.
    S("esc/panel-closes", "configstub.js", "kind=cm&esc=2800", has=["before: dialog=none detail=yes after: dialog=none detail=no"]),
    S("esc/panel-field-stays", "configstub.js", "kind=cm&esc=2800&escfrom=field", has=["from=field", "after: dialog=none detail=yes"]),
    S("esc/panel-select-closes", "configstub.js", "kind=cm&esc=2800&escfrom=select", has=["from=select", "after: dialog=none detail=no"]),
    S("esc/panel-unsaved-asks", "configstub.js", "kind=cm&edit=1&change=1&esc=2800",
      has=["after: dialog=Discard unsaved changes? detail=yes"]),
    # Modal's rules, once each (every dialog is a Modal, and must give it on_cancel):
    # cancels from a text field, only the dialog, not on a held key's repeats, not while busy.
    S("esc/delete", "stub.js", "pod=limited&ctx=prod&del=1&esc=dialog&escfrom=field",
      has=["from=field", "before: dialog=Delete Pod?", "after: dialog=none detail=yes"]),
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
    S("exec/window", "execstub.js", "exec=app", has=["Shell"]),
]


def build(out):
    subprocess.run(["trunk", "build", "--dist", out], cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    index = open(os.path.join(out, "index.html")).read()
    for f in os.listdir(HARNESS):
        if f.endswith(".js"):
            shutil.copy(os.path.join(HARNESS, f), out)
            # esc.js rides along with every stub (it does nothing without `esc=`); it has no page of its own.
            if f == "esc.js":
                continue
            page =index.replace("<head>", f'<head><script src="{f}"></script><script src="esc.js"></script>', 1)
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
