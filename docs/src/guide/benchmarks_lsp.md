### Measurement setup

- **Generated**: 2026-11-25T07:18:37.996816+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.31.0`
- **languageserver**: `languageserver 0.3.20; R 4.6.1; lintr 3.4.0`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"timing","points":[{"detail":"Median across fresh processes","document":"Initialize","tool":"arity","value":2.478},{"detail":"Median across fresh processes","document":"Initialize","tool":"languageserver","value":847.7249999999999},{"detail":"Median across fresh processes","document":"Initialize","tool":"ark","value":689.2909999999999},{"detail":"Median across fresh processes","document":"Workspace ready","tool":"arity","value":152.695},{"detail":"Median across fresh processes","document":"Workspace ready","tool":"languageserver","value":8311.171999999999},{"detail":"Median across fresh processes","document":"Workspace ready","tool":"ark","value":761.8660000000001},{"detail":"Median across fresh processes","document":"Open files ready","tool":"arity","value":489.204},{"detail":"Median across fresh processes","document":"Open files ready","tool":"languageserver","value":6123.976},{"detail":"Median across fresh processes","document":"Open files ready","tool":"ark","value":123.128},{"detail":"Median across fresh processes","document":"Edit workload","tool":"arity","value":9505.74},{"detail":"Median across fresh processes","document":"Edit workload","tool":"languageserver","value":31546.922},{"detail":"Median across fresh processes","document":"Edit workload","tool":"ark","value":3810.3379999999997}],"value_title":"Median elapsed time (ms)"}</script>
<figcaption>Median elapsed time (ms) on a logarithmic scale. Hover a point for measurements and sample details.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median elapsed time (ms)</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.478</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>847.725</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>689.291</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.695</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>8311.172</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.866</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>489.204</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>6123.976</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>123.128</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>9505.740</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>31546.922</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3810.338</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"timing","points":[{"detail":"p95: 0.965 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0","document":"Document symbols","tool":"arity","value":0.554},{"detail":"p95: 0.744 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","tool":"languageserver","value":0.605},{"detail":"p95: 0.719 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","tool":"ark","value":0.559},{"detail":"p95: 0.335 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0","document":"Hover","tool":"arity","value":0.252},{"detail":"p95: 7.119 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","tool":"languageserver","value":5.449},{"detail":"p95: 15.510 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","tool":"ark","value":11.339},{"detail":"p95: 0.094 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","tool":"arity","value":0.083},{"detail":"p95: 1.235 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","tool":"languageserver","value":1.165},{"detail":"p95: 0.169 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","tool":"ark","value":0.121},{"detail":"p95: 0.105 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","tool":"arity","value":0.078},{"detail":"p95: 3.226 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","tool":"languageserver","value":2.863},{"detail":"p95: 0.228 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","tool":"ark","value":0.212},{"detail":"p95: 0.103 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","tool":"arity","value":0.084},{"detail":"p95: 3.366 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","tool":"languageserver","value":2.955},{"detail":"p95: 0.244 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","tool":"ark","value":0.217},{"detail":"p95: 0.601 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0","document":"Completion","tool":"arity","value":0.498},{"detail":"p95: 6.616 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 500–500; stale responses: 0","document":"Completion","tool":"languageserver","value":5.62},{"detail":"p95: 30.162 ms; samples: 60; empty: 0; returned items: 2607–2607; result bytes: 711090–711090; stale responses: 0","document":"Completion","tool":"ark","value":22.914},{"detail":"p95: 11.973 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0","document":"Signature help","tool":"arity","value":6.496},{"detail":"p95: 7.773 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0","document":"Signature help","tool":"languageserver","value":6.632},{"detail":"p95: 21.395 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0","document":"Signature help","tool":"ark","value":9.335},{"detail":"p95: 0.181 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","tool":"arity","value":0.122},{"detail":"p95: 8.096 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0","document":"Workspace symbols","tool":"languageserver","value":4.485},{"detail":"p95: 0.133 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0","document":"Workspace symbols","tool":"ark","value":0.083},{"detail":"p95: 3.320 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0","document":"Folding ranges","tool":"arity","value":1.075},{"detail":"p95: 0.700 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0","document":"Folding ranges","tool":"languageserver","value":0.523},{"detail":"p95: 37.879 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0","document":"Folding ranges","tool":"ark","value":36.52},{"detail":"p95: 3.716 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0","document":"Project references","tool":"arity","value":2.806},{"detail":"p95: 2.658 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0","document":"Project references","tool":"languageserver","value":2.218},{"detail":"p95: 2.226 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0","document":"Project references","tool":"ark","value":2.165},{"detail":"p95: 10.391 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","tool":"arity","value":9.351},{"detail":"p95: 39.814 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 54","document":"Edit to definition","tool":"languageserver","value":28.949},{"detail":"p95: 5.156 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","tool":"ark","value":3.599}],"value_title":"Median latency (ms)"}</script>
<figcaption>Median latency (ms) on a logarithmic scale. Hover a point for measurements and sample details.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median latency (ms)</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>0.554</td><td>p95: 0.965 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>0.605</td><td>p95: 0.744 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.559</td><td>p95: 0.719 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.252</td><td>p95: 0.335 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>5.449</td><td>p95: 7.119 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.339</td><td>p95: 15.510 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.083</td><td>p95: 0.094 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>1.165</td><td>p95: 1.235 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.121</td><td>p95: 0.169 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>0.078</td><td>p95: 0.105 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>2.863</td><td>p95: 3.226 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.212</td><td>p95: 0.228 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>0.084</td><td>p95: 0.103 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>2.955</td><td>p95: 3.366 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.217</td><td>p95: 0.244 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Completion</td><td>arity</td><td>0.498</td><td>p95: 0.601 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 341–341; stale responses: 0</td></tr>
<tr><td>Completion</td><td>languageserver</td><td>5.620</td><td>p95: 6.616 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 500–500; stale responses: 0</td></tr>
<tr><td>Completion</td><td>ark</td><td>22.914</td><td>p95: 30.162 ms; samples: 60; empty: 0; returned items: 2607–2607; result bytes: 711090–711090; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>arity</td><td>6.496</td><td>p95: 11.973 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 448–448; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>languageserver</td><td>6.632</td><td>p95: 7.773 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 229–229; stale responses: 0</td></tr>
<tr><td>Signature help</td><td>ark</td><td>9.335</td><td>p95: 21.395 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 459–459; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>arity</td><td>0.122</td><td>p95: 0.181 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>languageserver</td><td>4.485</td><td>p95: 8.096 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 647–647; stale responses: 0</td></tr>
<tr><td>Workspace symbols</td><td>ark</td><td>0.083</td><td>p95: 0.133 ms; samples: 60; empty: 0; returned items: 3–3; result bytes: 650–650; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>arity</td><td>1.075</td><td>p95: 3.320 ms; samples: 60; empty: 0; returned items: 98–98; result bytes: 3237–3237; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>languageserver</td><td>0.523</td><td>p95: 0.700 ms; samples: 60; empty: 0; returned items: 96–96; result bytes: 4613–4613; stale responses: 0</td></tr>
<tr><td>Folding ranges</td><td>ark</td><td>36.520</td><td>p95: 37.879 ms; samples: 60; empty: 0; returned items: 93–93; result bytes: 6380–6380; stale responses: 0</td></tr>
<tr><td>Project references</td><td>arity</td><td>2.806</td><td>p95: 3.716 ms; samples: 60; empty: 0; returned items: 68–68; result bytes: 12484–12484; stale responses: 0</td></tr>
<tr><td>Project references</td><td>languageserver</td><td>2.218</td><td>p95: 2.658 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 168–168; stale responses: 0</td></tr>
<tr><td>Project references</td><td>ark</td><td>2.165</td><td>p95: 2.226 ms; samples: 60; empty: 0; returned items: 65–65; result bytes: 11996–11996; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>9.351</td><td>p95: 10.391 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>28.949</td><td>p95: 39.814 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 54</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.599</td><td>p95: 5.156 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 26.5 MiB; processes: 1","document":"Initialized","tool":"arity","value":28.3},{"detail":"PSS: 690.6 MiB; processes: 6","document":"Initialized","tool":"languageserver","value":801.9},{"detail":"PSS: 111.2 MiB; processes: 1","document":"Initialized","tool":"ark","value":120.4},{"detail":"PSS: 357.5 MiB; processes: 1","document":"Files open","tool":"arity","value":359.4},{"detail":"PSS: 978.7 MiB; processes: 7","document":"Files open","tool":"languageserver","value":1112.4},{"detail":"PSS: 116.4 MiB; processes: 1","document":"Files open","tool":"ark","value":125.6},{"detail":"PSS: 454.2 MiB; processes: 1","document":"After edits","tool":"arity","value":456.1},{"detail":"PSS: 1267.3 MiB; processes: 7","document":"After edits","tool":"languageserver","value":1400.8},{"detail":"PSS: 222.8 MiB; processes: 1","document":"After edits","tool":"ark","value":232.2},{"detail":"PSS: 508.2 MiB; processes: 2","document":"Sampled peak","tool":"arity","value":510.1},{"detail":"PSS: 1267.3 MiB; processes: 10","document":"Sampled peak","tool":"languageserver","value":1400.8},{"detail":"PSS: 222.8 MiB; processes: 1","document":"Sampled peak","tool":"ark","value":232.2}],"value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage. Hover a bar for PSS and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>28.300</td><td>PSS: 26.5 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>801.900</td><td>PSS: 690.6 MiB; processes: 6</td></tr>
<tr><td>Initialized</td><td>ark</td><td>120.400</td><td>PSS: 111.2 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>359.400</td><td>PSS: 357.5 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1112.400</td><td>PSS: 978.7 MiB; processes: 7</td></tr>
<tr><td>Files open</td><td>ark</td><td>125.600</td><td>PSS: 116.4 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>456.100</td><td>PSS: 454.2 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>1400.800</td><td>PSS: 1267.3 MiB; processes: 7</td></tr>
<tr><td>After edits</td><td>ark</td><td>232.200</td><td>PSS: 222.8 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>510.100</td><td>PSS: 508.2 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>1400.800</td><td>PSS: 1267.3 MiB; processes: 10</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>232.200</td><td>PSS: 222.8 MiB; processes: 1</td></tr>
</tbody>
</table>
</details>
</div>
