### Measurement setup

- **Generated**: 2026-09-20T20:34:06.405999+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.23.0`
- **languageserver**: `languageserver 0.3.18; R 4.6.1; lintr 3.3.0.1`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.231},{"detail":"Median across fresh processes","document":"Initialize","ratio":414.04034065441505,"ratio_label":"414.04× arity","tool":"languageserver","value":923.7239999999999},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.227},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":29.61313039079795,"ratio_label":"29.61× arity","tool":"languageserver","value":4507.918},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":336.303},{"detail":"Median across fresh processes","document":"Open files ready","ratio":5.5232216185998935,"ratio_label":"5.52× arity","tool":"languageserver","value":1857.4759999999999},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2609.499},{"detail":"Median across fresh processes","document":"Edit workload","ratio":18.049436692637173,"ratio_label":"18.05× arity","tool":"languageserver","value":47099.987}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.231</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>923.724</td><td>414.04× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.227</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>4507.918</td><td>29.61× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>336.303</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>1857.476</td><td>5.52× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>2609.499</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>47099.987</td><td>18.05× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 2.130 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.522},{"detail":"p95: 36.145 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":19.00722733245729,"ratio_label":"19.01× arity","tool":"languageserver","value":28.929},{"detail":"p95: 0.823 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.672},{"detail":"p95: 21.518 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":28.87053571428571,"ratio_label":"28.87× arity","tool":"languageserver","value":19.401},{"detail":"p95: 1.347 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.128},{"detail":"p95: 19.827 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":16.10372340425532,"ratio_label":"16.10× arity","tool":"languageserver","value":18.165},{"detail":"p95: 1.324 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.093},{"detail":"p95: 50.124 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":41.86916742909423,"ratio_label":"41.87× arity","tool":"languageserver","value":45.763},{"detail":"p95: 1.395 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.055},{"detail":"p95: 51.622 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":44.71848341232227,"ratio_label":"44.72× arity","tool":"languageserver","value":47.178},{"detail":"p95: 3.701 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.419},{"detail":"p95: 96.456 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16","document":"Edit to definition","ratio":16.400992145514675,"ratio_label":"16.40× arity","tool":"languageserver","value":39.674}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>1.522</td><td>baseline</td><td>p95: 2.130 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>28.929</td><td>19.01× arity</td><td>p95: 36.145 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.672</td><td>baseline</td><td>p95: 0.823 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>19.401</td><td>28.87× arity</td><td>p95: 21.518 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>1.128</td><td>baseline</td><td>p95: 1.347 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>18.165</td><td>16.10× arity</td><td>p95: 19.827 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>1.093</td><td>baseline</td><td>p95: 1.324 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>45.763</td><td>41.87× arity</td><td>p95: 50.124 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>1.055</td><td>baseline</td><td>p95: 1.395 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>47.178</td><td>44.72× arity</td><td>p95: 51.622 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>2.419</td><td>baseline</td><td>p95: 3.701 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>39.674</td><td>16.40× arity</td><td>p95: 96.456 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 23.8 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":25.6},{"detail":"PSS: 899.1 MiB; processes: 9","document":"Initialized","ratio":40.96093749999999,"ratio_label":"40.96× arity","tool":"languageserver","value":1048.6},{"detail":"PSS: 328.8 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":330.6},{"detail":"PSS: 1526.2 MiB; processes: 14","document":"Files open","ratio":5.349969751966122,"ratio_label":"5.35× arity","tool":"languageserver","value":1768.7},{"detail":"PSS: 372.6 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":374.5},{"detail":"PSS: 823.6 MiB; processes: 3","document":"After edits","ratio":2.3148197596795725,"ratio_label":"2.31× arity","tool":"languageserver","value":866.9},{"detail":"PSS: 399.6 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":401.5},{"detail":"PSS: 1779.7 MiB; processes: 15","document":"Sampled peak","ratio":5.036861768368618,"ratio_label":"5.04× arity","tool":"languageserver","value":2022.3}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>25.600</td><td>baseline</td><td>PSS: 23.8 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1048.600</td><td>40.96× arity</td><td>PSS: 899.1 MiB; processes: 9</td></tr>
<tr><td>Files open</td><td>arity</td><td>330.600</td><td>baseline</td><td>PSS: 328.8 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1768.700</td><td>5.35× arity</td><td>PSS: 1526.2 MiB; processes: 14</td></tr>
<tr><td>After edits</td><td>arity</td><td>374.500</td><td>baseline</td><td>PSS: 372.6 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>866.900</td><td>2.31× arity</td><td>PSS: 823.6 MiB; processes: 3</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>401.500</td><td>baseline</td><td>PSS: 399.6 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2022.300</td><td>5.04× arity</td><td>PSS: 1779.7 MiB; processes: 15</td></tr>
</tbody>
</table>
</details>
</div>
