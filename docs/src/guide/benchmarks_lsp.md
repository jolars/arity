### Measurement setup

- **Generated**: 2026-09-28T18:52:10.513844+00:00
- **Host**: Linux/x86_64, AMD Ryzen 9 7900 12-Core Processor
- **Corpus**: tidyr v1.3.2 at `bfdfc359b90b5f1432eb9de21cd03d9c11893137`
- **Open buffers**: 5 files, 82999 bytes including benchmark bindings
- **Workload**: 3 fresh processes per server, 1000 edits per process
- **Warm requests**: 2 warmups and 20 measured rounds per target per process
- **Settling**: 5 seconds below 5% of one CPU core, sampled every 0.15 seconds
- **arity**: `arity 0.24.0`
- **languageserver**: `languageserver 0.3.18; R 4.6.1; lintr 3.3.0.1`
- **ark**: `Ark 0.1.252; R version 4.6.1 (2026-06-24)`

Opened files: `R/pivot-wide.R`, `R/separate-wider.R`, `R/pivot-long.R`, `R/import-standalone-types-check.R`, `R/chop.R`.

### Startup and workload

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.639},{"detail":"Median across fresh processes","document":"Initialize","ratio":358.3251231527094,"ratio_label":"358.33× arity","tool":"languageserver","value":945.62},{"detail":"Median across fresh processes","document":"Initialize","ratio":235.19628647214856,"ratio_label":"235.20× arity","tool":"ark","value":620.683},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.625},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":28.495004095004095,"ratio_label":"28.50× arity","tool":"languageserver","value":4349.05},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":4.991521703521704,"ratio_label":"4.99× arity","tool":"ark","value":761.831},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":486.94399999999996},{"detail":"Median across fresh processes","document":"Open files ready","ratio":3.8243411973450745,"ratio_label":"3.82× arity","tool":"languageserver","value":1862.2399999999998},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.20821080042058226,"ratio_label":"0.21× arity","tool":"ark","value":101.387},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2438.893},{"detail":"Median across fresh processes","document":"Edit workload","ratio":18.800965027986056,"ratio_label":"18.80× arity","tool":"languageserver","value":45853.541999999994},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.6023810802687941,"ratio_label":"1.60× arity","tool":"ark","value":3908.036}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.639</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>945.620</td><td>358.33× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>620.683</td><td>235.20× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.625</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>4349.050</td><td>28.50× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.831</td><td>4.99× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>486.944</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>1862.240</td><td>3.82× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>101.387</td><td>0.21× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>2438.893</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>45853.542</td><td>18.80× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3908.036</td><td>1.60× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 4.984 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.532},{"detail":"p95: 39.159 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":19.57832898172324,"ratio_label":"19.58× arity","tool":"languageserver","value":29.994},{"detail":"p95: 0.628 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":0.3198433420365535,"ratio_label":"0.32× arity","tool":"ark","value":0.49},{"detail":"p95: 1.814 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.704},{"detail":"p95: 20.015 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":26.529829545454547,"ratio_label":"26.53× arity","tool":"languageserver","value":18.677},{"detail":"p95: 13.036 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":16.588068181818183,"ratio_label":"16.59× arity","tool":"ark","value":11.678},{"detail":"p95: 1.547 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.166},{"detail":"p95: 19.998 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":15.168953687821615,"ratio_label":"15.17× arity","tool":"languageserver","value":17.687},{"detail":"p95: 0.148 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":0.08404802744425387,"ratio_label":"0.08× arity","tool":"ark","value":0.098},{"detail":"p95: 1.460 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.186},{"detail":"p95: 47.467 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":37.52445193929174,"ratio_label":"37.52× arity","tool":"languageserver","value":44.504},{"detail":"p95: 0.230 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":0.17284991568296795,"ratio_label":"0.17× arity","tool":"ark","value":0.205},{"detail":"p95: 1.362 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.106},{"detail":"p95: 49.306 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":41.371609403254965,"ratio_label":"41.37× arity","tool":"languageserver","value":45.757},{"detail":"p95: 0.248 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":0.1880650994575045,"ratio_label":"0.19× arity","tool":"ark","value":0.208},{"detail":"p95: 3.518 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.211},{"detail":"p95: 92.131 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 17","document":"Edit to definition","ratio":17.545454545454547,"ratio_label":"17.55× arity","tool":"languageserver","value":38.793},{"detail":"p95: 5.020 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":1.6472184531886025,"ratio_label":"1.65× arity","tool":"ark","value":3.642}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>1.532</td><td>baseline</td><td>p95: 4.984 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>29.994</td><td>19.58× arity</td><td>p95: 39.159 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.490</td><td>0.32× arity</td><td>p95: 0.628 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.704</td><td>baseline</td><td>p95: 1.814 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>18.677</td><td>26.53× arity</td><td>p95: 20.015 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.678</td><td>16.59× arity</td><td>p95: 13.036 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>1.166</td><td>baseline</td><td>p95: 1.547 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>17.687</td><td>15.17× arity</td><td>p95: 19.998 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.098</td><td>0.08× arity</td><td>p95: 0.148 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>1.186</td><td>baseline</td><td>p95: 1.460 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>44.504</td><td>37.52× arity</td><td>p95: 47.467 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.205</td><td>0.17× arity</td><td>p95: 0.230 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>1.106</td><td>baseline</td><td>p95: 1.362 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>45.757</td><td>41.37× arity</td><td>p95: 49.306 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.208</td><td>0.19× arity</td><td>p95: 0.248 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>2.211</td><td>baseline</td><td>p95: 3.518 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>38.793</td><td>17.55× arity</td><td>p95: 92.131 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 17</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.642</td><td>1.65× arity</td><td>p95: 5.020 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 23.8 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":25.6},{"detail":"PSS: 899.0 MiB; processes: 9","document":"Initialized","ratio":41.03125,"ratio_label":"41.03× arity","tool":"languageserver","value":1050.4},{"detail":"PSS: 110.9 MiB; processes: 1","document":"Initialized","ratio":4.66015625,"ratio_label":"4.66× arity","tool":"ark","value":119.3},{"detail":"PSS: 320.3 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":322.2},{"detail":"PSS: 1526.4 MiB; processes: 14","document":"Files open","ratio":5.499068901303538,"ratio_label":"5.50× arity","tool":"languageserver","value":1771.8},{"detail":"PSS: 115.8 MiB; processes: 1","document":"Files open","ratio":0.3857852265673495,"ratio_label":"0.39× arity","tool":"ark","value":124.3},{"detail":"PSS: 402.4 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":404.3},{"detail":"PSS: 822.8 MiB; processes: 3","document":"After edits","ratio":2.144694533762058,"ratio_label":"2.14× arity","tool":"languageserver","value":867.1},{"detail":"PSS: 124.1 MiB; processes: 1","document":"After edits","ratio":0.3284689586940391,"ratio_label":"0.33× arity","tool":"ark","value":132.8},{"detail":"PSS: 410.3 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":412.2},{"detail":"PSS: 1778.5 MiB; processes: 15","document":"Sampled peak","ratio":4.910237748665696,"ratio_label":"4.91× arity","tool":"languageserver","value":2024.0},{"detail":"PSS: 124.1 MiB; processes: 2","document":"Sampled peak","ratio":0.32217370208636587,"ratio_label":"0.32× arity","tool":"ark","value":132.8}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>25.600</td><td>baseline</td><td>PSS: 23.8 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1050.400</td><td>41.03× arity</td><td>PSS: 899.0 MiB; processes: 9</td></tr>
<tr><td>Initialized</td><td>ark</td><td>119.300</td><td>4.66× arity</td><td>PSS: 110.9 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>322.200</td><td>baseline</td><td>PSS: 320.3 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1771.800</td><td>5.50× arity</td><td>PSS: 1526.4 MiB; processes: 14</td></tr>
<tr><td>Files open</td><td>ark</td><td>124.300</td><td>0.39× arity</td><td>PSS: 115.8 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>404.300</td><td>baseline</td><td>PSS: 402.4 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>867.100</td><td>2.14× arity</td><td>PSS: 822.8 MiB; processes: 3</td></tr>
<tr><td>After edits</td><td>ark</td><td>132.800</td><td>0.33× arity</td><td>PSS: 124.1 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>412.200</td><td>baseline</td><td>PSS: 410.3 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2024.000</td><td>4.91× arity</td><td>PSS: 1778.5 MiB; processes: 15</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>132.800</td><td>0.32× arity</td><td>PSS: 124.1 MiB; processes: 2</td></tr>
</tbody>
</table>
</details>
</div>
