### Measurement setup

- **Generated**: 2026-11-20T21:12:50.297610+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.31.0`
- **languageserver**: `languageserver 0.3.18; R 4.6.1; lintr 3.3.0.1`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.614},{"detail":"Median across fresh processes","document":"Initialize","ratio":371.35348125478197,"ratio_label":"371.35× arity","tool":"languageserver","value":970.718},{"detail":"Median across fresh processes","document":"Initialize","ratio":251.1805661820964,"ratio_label":"251.18× arity","tool":"ark","value":656.586},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.53300000000002},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":29.45017143831171,"ratio_label":"29.45× arity","tool":"languageserver","value":4492.1230000000005},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":4.991798496063146,"ratio_label":"4.99× arity","tool":"ark","value":761.414},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1399.47},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.3778659063788434,"ratio_label":"1.38× arity","tool":"languageserver","value":1928.2820000000002},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.026319249430141408,"ratio_label":"0.03× arity","tool":"ark","value":36.833},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":14941.802},{"detail":"Median across fresh processes","document":"Edit workload","ratio":3.1335899110428582,"ratio_label":"3.13× arity","tool":"languageserver","value":46821.48},{"detail":"Median across fresh processes","document":"Edit workload","ratio":0.26336395034548044,"ratio_label":"0.26× arity","tool":"ark","value":3935.132}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.614</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>970.718</td><td>371.35× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>656.586</td><td>251.18× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.533</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>4492.123</td><td>29.45× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.414</td><td>4.99× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>1399.470</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>1928.282</td><td>1.38× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>36.833</td><td>0.03× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>14941.802</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>46821.480</td><td>3.13× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3935.132</td><td>0.26× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 259.132 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":248.859},{"detail":"p95: 35.522 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":0.11481602031672554,"ratio_label":"0.11× arity","tool":"languageserver","value":28.573},{"detail":"p95: 0.607 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":0.0019569314350696577,"ratio_label":"0.00× arity","tool":"ark","value":0.487},{"detail":"p95: 10.517 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":7.938},{"detail":"p95: 20.457 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":2.3214915595867978,"ratio_label":"2.32× arity","tool":"languageserver","value":18.428},{"detail":"p95: 12.298 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":1.4040060468631896,"ratio_label":"1.40× arity","tool":"ark","value":11.145},{"detail":"p95: 0.912 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.637},{"detail":"p95: 19.860 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":27.808477237048663,"ratio_label":"27.81× arity","tool":"languageserver","value":17.714},{"detail":"p95: 0.174 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":0.1554160125588697,"ratio_label":"0.16× arity","tool":"ark","value":0.099},{"detail":"p95: 47.332 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":44.745},{"detail":"p95: 48.019 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":0.9848027712593586,"ratio_label":"0.98× arity","tool":"languageserver","value":44.065},{"detail":"p95: 0.253 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":0.005162587998659068,"ratio_label":"0.01× arity","tool":"ark","value":0.231},{"detail":"p95: 17.618 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":5.564},{"detail":"p95: 47.285 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":8.012580877066858,"ratio_label":"8.01× arity","tool":"languageserver","value":44.582},{"detail":"p95: 0.256 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":0.040977713874910136,"ratio_label":"0.04× arity","tool":"ark","value":0.228},{"detail":"p95: 16.196 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":14.419},{"detail":"p95: 94.192 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16","document":"Edit to definition","ratio":2.7307025452527913,"ratio_label":"2.73× arity","tool":"languageserver","value":39.374},{"detail":"p95: 4.901 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":0.2573687495665441,"ratio_label":"0.26× arity","tool":"ark","value":3.711}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>248.859</td><td>baseline</td><td>p95: 259.132 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 14064–25473; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>28.573</td><td>0.11× arity</td><td>p95: 35.522 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.487</td><td>0.00× arity</td><td>p95: 0.607 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>7.938</td><td>baseline</td><td>p95: 10.517 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 224–224; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>18.428</td><td>2.32× arity</td><td>p95: 20.457 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.145</td><td>1.40× arity</td><td>p95: 12.298 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.637</td><td>baseline</td><td>p95: 0.912 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>17.714</td><td>27.81× arity</td><td>p95: 19.860 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.099</td><td>0.16× arity</td><td>p95: 0.174 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>44.745</td><td>baseline</td><td>p95: 47.332 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>44.065</td><td>0.98× arity</td><td>p95: 48.019 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.231</td><td>0.01× arity</td><td>p95: 0.253 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>5.564</td><td>baseline</td><td>p95: 17.618 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>44.582</td><td>8.01× arity</td><td>p95: 47.285 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.228</td><td>0.04× arity</td><td>p95: 0.256 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>14.419</td><td>baseline</td><td>p95: 16.196 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>39.374</td><td>2.73× arity</td><td>p95: 94.192 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 16</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.711</td><td>0.26× arity</td><td>p95: 4.901 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 26.3 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":27.9},{"detail":"PSS: 902.6 MiB; processes: 9","document":"Initialized","ratio":37.971326164874554,"ratio_label":"37.97× arity","tool":"languageserver","value":1059.4},{"detail":"PSS: 110.6 MiB; processes: 1","document":"Initialized","ratio":4.311827956989247,"ratio_label":"4.31× arity","tool":"ark","value":120.3},{"detail":"PSS: 284.9 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":286.6},{"detail":"PSS: 1530.6 MiB; processes: 14","document":"Files open","ratio":6.230983949755757,"ratio_label":"6.23× arity","tool":"languageserver","value":1785.8},{"detail":"PSS: 115.8 MiB; processes: 1","document":"Files open","ratio":0.4378925331472435,"ratio_label":"0.44× arity","tool":"ark","value":125.5},{"detail":"PSS: 429.3 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":430.8},{"detail":"PSS: 816.3 MiB; processes: 3","document":"After edits","ratio":2.0041782729805013,"ratio_label":"2.00× arity","tool":"languageserver","value":863.4},{"detail":"PSS: 123.6 MiB; processes: 1","document":"After edits","ratio":0.3108170844939647,"ratio_label":"0.31× arity","tool":"ark","value":133.9},{"detail":"PSS: 529.7 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":531.1},{"detail":"PSS: 1775.7 MiB; processes: 15","document":"Sampled peak","ratio":3.824138580305027,"ratio_label":"3.82× arity","tool":"languageserver","value":2031.0},{"detail":"PSS: 123.9 MiB; processes: 2","document":"Sampled peak","ratio":0.2521182451515722,"ratio_label":"0.25× arity","tool":"ark","value":133.9}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>27.900</td><td>baseline</td><td>PSS: 26.3 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1059.400</td><td>37.97× arity</td><td>PSS: 902.6 MiB; processes: 9</td></tr>
<tr><td>Initialized</td><td>ark</td><td>120.300</td><td>4.31× arity</td><td>PSS: 110.6 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>286.600</td><td>baseline</td><td>PSS: 284.9 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1785.800</td><td>6.23× arity</td><td>PSS: 1530.6 MiB; processes: 14</td></tr>
<tr><td>Files open</td><td>ark</td><td>125.500</td><td>0.44× arity</td><td>PSS: 115.8 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>430.800</td><td>baseline</td><td>PSS: 429.3 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>863.400</td><td>2.00× arity</td><td>PSS: 816.3 MiB; processes: 3</td></tr>
<tr><td>After edits</td><td>ark</td><td>133.900</td><td>0.31× arity</td><td>PSS: 123.6 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>531.100</td><td>baseline</td><td>PSS: 529.7 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2031.000</td><td>3.82× arity</td><td>PSS: 1775.7 MiB; processes: 15</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>133.900</td><td>0.25× arity</td><td>PSS: 123.9 MiB; processes: 2</td></tr>
</tbody>
</table>
</details>
</div>
