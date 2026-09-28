### Measurement setup

- **Generated**: 2026-09-28T20:27:32.650494+00:00
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
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"Median across fresh processes","document":"Initialize","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":2.0660000000000003},{"detail":"Median across fresh processes","document":"Initialize","ratio":480.17618586640845,"ratio_label":"480.18× arity","tool":"languageserver","value":992.044},{"detail":"Median across fresh processes","document":"Initialize","ratio":338.7270087124879,"ratio_label":"338.73× arity","tool":"ark","value":699.8100000000001},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":152.225},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":30.71078994908852,"ratio_label":"30.71× arity","tool":"languageserver","value":4674.95},{"detail":"Median across fresh processes","document":"Workspace ready","ratio":5.004880932829693,"ratio_label":"5.00× arity","tool":"ark","value":761.8679999999999},{"detail":"Median across fresh processes","document":"Open files ready","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":483.728},{"detail":"Median across fresh processes","document":"Open files ready","ratio":3.8249822214136873,"ratio_label":"3.82× arity","tool":"languageserver","value":1850.2510000000002},{"detail":"Median across fresh processes","document":"Open files ready","ratio":0.2504713392650415,"ratio_label":"0.25× arity","tool":"ark","value":121.16000000000001},{"detail":"Median across fresh processes","document":"Edit workload","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1681.057},{"detail":"Median across fresh processes","document":"Edit workload","ratio":27.297189803796062,"ratio_label":"27.30× arity","tool":"languageserver","value":45888.132},{"detail":"Median across fresh processes","document":"Edit workload","ratio":2.3138257655748737,"ratio_label":"2.31× arity","tool":"ark","value":3889.6730000000002}],"ratio_title":"Time relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Time relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialize</td><td>arity</td><td>2.066</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>languageserver</td><td>992.044</td><td>480.18× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Initialize</td><td>ark</td><td>699.810</td><td>338.73× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>arity</td><td>152.225</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>languageserver</td><td>4674.950</td><td>30.71× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Workspace ready</td><td>ark</td><td>761.868</td><td>5.00× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>arity</td><td>483.728</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>languageserver</td><td>1850.251</td><td>3.82× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Open files ready</td><td>ark</td><td>121.160</td><td>0.25× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>arity</td><td>1681.057</td><td>baseline</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>languageserver</td><td>45888.132</td><td>27.30× arity</td><td>Median across fresh processes</td></tr>
<tr><td>Edit workload</td><td>ark</td><td>3889.673</td><td>2.31× arity</td><td>Median across fresh processes</td></tr>
</tbody>
</table>
</details>
</div>

### Request latency

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"ratio","points":[{"detail":"p95: 0.810 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0","document":"Document symbols","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.544},{"detail":"p95: 36.417 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0","document":"Document symbols","ratio":54.86397058823529,"ratio_label":"54.86× arity","tool":"languageserver","value":29.846},{"detail":"p95: 0.639 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0","document":"Document symbols","ratio":0.9669117647058824,"ratio_label":"0.97× arity","tool":"ark","value":0.526},{"detail":"p95: 0.958 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0","document":"Hover","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.749},{"detail":"p95: 19.743 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0","document":"Hover","ratio":24.65554072096128,"ratio_label":"24.66× arity","tool":"languageserver","value":18.467},{"detail":"p95: 14.163 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0","document":"Hover","ratio":15.945260347129505,"ratio_label":"15.95× arity","tool":"ark","value":11.943},{"detail":"p95: 0.085 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.066},{"detail":"p95: 19.716 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Go to definition","ratio":268.6363636363636,"ratio_label":"268.64× arity","tool":"languageserver","value":17.73},{"detail":"p95: 0.162 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Go to definition","ratio":1.5606060606060606,"ratio_label":"1.56× arity","tool":"ark","value":0.103},{"detail":"p95: 0.091 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.07},{"detail":"p95: 48.012 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":641.3571428571429,"ratio_label":"641.36× arity","tool":"languageserver","value":44.895},{"detail":"p95: 0.310 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0","document":"Find references","ratio":2.9428571428571426,"ratio_label":"2.94× arity","tool":"ark","value":0.206},{"detail":"p95: 0.092 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":0.059},{"detail":"p95: 48.880 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":763.4745762711865,"ratio_label":"763.47× arity","tool":"languageserver","value":45.045},{"detail":"p95: 0.264 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0","document":"Rename","ratio":3.6610169491525424,"ratio_label":"3.66× arity","tool":"ark","value":0.216},{"detail":"p95: 2.317 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0","document":"Edit to definition","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":1.54},{"detail":"p95: 95.213 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 18","document":"Edit to definition","ratio":25.069480519480518,"ratio_label":"25.07× arity","tool":"languageserver","value":38.607},{"detail":"p95: 4.945 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0","document":"Edit to definition","ratio":2.3805194805194803,"ratio_label":"2.38× arity","tool":"ark","value":3.666}],"ratio_title":"Latency relative to arity","value_title":"Median (ms)"}</script>
<figcaption>Latency relative to arity. Hover a point for measurements and sample details. Lower values use less time.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median (ms)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Document symbols</td><td>arity</td><td>0.544</td><td>baseline</td><td>p95: 0.810 ms; samples: 180; empty: 0; returned items: 66–120; result bytes: 12975–23793; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>languageserver</td><td>29.846</td><td>54.86× arity</td><td>p95: 36.417 ms; samples: 180; empty: 0; returned items: 6–17; result bytes: 1217–3477; stale responses: 0</td></tr>
<tr><td>Document symbols</td><td>ark</td><td>0.526</td><td>0.97× arity</td><td>p95: 0.639 ms; samples: 180; empty: 0; returned items: 6–18; result bytes: 2031–5186; stale responses: 0</td></tr>
<tr><td>Hover</td><td>arity</td><td>0.749</td><td>baseline</td><td>p95: 0.958 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 811–811; stale responses: 0</td></tr>
<tr><td>Hover</td><td>languageserver</td><td>18.467</td><td>24.66× arity</td><td>p95: 19.743 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1880–1880; stale responses: 0</td></tr>
<tr><td>Hover</td><td>ark</td><td>11.943</td><td>15.95× arity</td><td>p95: 14.163 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 1675–1675; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>arity</td><td>0.066</td><td>baseline</td><td>p95: 0.085 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>languageserver</td><td>17.730</td><td>268.64× arity</td><td>p95: 19.716 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Go to definition</td><td>ark</td><td>0.103</td><td>1.56× arity</td><td>p95: 0.162 ms; samples: 60; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
<tr><td>Find references</td><td>arity</td><td>0.070</td><td>baseline</td><td>p95: 0.091 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>languageserver</td><td>44.895</td><td>641.36× arity</td><td>p95: 48.012 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Find references</td><td>ark</td><td>0.206</td><td>2.94× arity</td><td>p95: 0.310 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 335–335; stale responses: 0</td></tr>
<tr><td>Rename</td><td>arity</td><td>0.059</td><td>baseline</td><td>p95: 0.092 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>languageserver</td><td>45.045</td><td>763.47× arity</td><td>p95: 48.880 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Rename</td><td>ark</td><td>0.216</td><td>3.66× arity</td><td>p95: 0.264 ms; samples: 60; empty: 0; returned items: 2–2; result bytes: 317–317; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>arity</td><td>1.540</td><td>baseline</td><td>p95: 2.317 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 0</td></tr>
<tr><td>Edit to definition</td><td>languageserver</td><td>38.607</td><td>25.07× arity</td><td>p95: 95.213 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 166–166; stale responses: 18</td></tr>
<tr><td>Edit to definition</td><td>ark</td><td>3.666</td><td>2.38× arity</td><td>p95: 4.945 ms; samples: 3000; empty: 0; returned items: 1–1; result bytes: 274–274; stale responses: 0</td></tr>
</tbody>
</table>
</details>
</div>

### Resident memory

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">{"kind":"bar","points":[{"detail":"PSS: 24.1 MiB; processes: 1","document":"Initialized","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":25.9},{"detail":"PSS: 899.3 MiB; processes: 9","document":"Initialized","ratio":40.57915057915058,"ratio_label":"40.58× arity","tool":"languageserver","value":1051.0},{"detail":"PSS: 111.0 MiB; processes: 1","document":"Initialized","ratio":4.6061776061776065,"ratio_label":"4.61× arity","tool":"ark","value":119.3},{"detail":"PSS: 316.5 MiB; processes: 1","document":"Files open","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":318.3},{"detail":"PSS: 1526.8 MiB; processes: 14","document":"Files open","ratio":5.569588438579956,"ratio_label":"5.57× arity","tool":"languageserver","value":1772.8},{"detail":"PSS: 116.0 MiB; processes: 1","document":"Files open","ratio":0.3901979264844486,"ratio_label":"0.39× arity","tool":"ark","value":124.2},{"detail":"PSS: 339.7 MiB; processes: 1","document":"After edits","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":341.6},{"detail":"PSS: 823.6 MiB; processes: 3","document":"After edits","ratio":2.5401053864168617,"ratio_label":"2.54× arity","tool":"languageserver","value":867.7},{"detail":"PSS: 124.3 MiB; processes: 1","document":"After edits","ratio":0.3890515222482436,"ratio_label":"0.39× arity","tool":"ark","value":132.9},{"detail":"PSS: 359.8 MiB; processes: 2","document":"Sampled peak","ratio":1.0,"ratio_label":"baseline","tool":"arity","value":361.6},{"detail":"PSS: 1779.4 MiB; processes: 15","document":"Sampled peak","ratio":5.6009402654867255,"ratio_label":"5.60× arity","tool":"languageserver","value":2025.3},{"detail":"PSS: 124.3 MiB; processes: 2","document":"Sampled peak","ratio":0.36753318584070793,"ratio_label":"0.37× arity","tool":"ark","value":132.9}],"ratio_title":"Memory relative to arity","value_title":"Median RSS (MiB)"}</script>
<figcaption>Median RSS (MiB) at each session stage, with one bar per server on a linear scale starting at zero. Hover a bar for relative memory use, PSS, and process counts.</figcaption>
</figure>
<details class="bench-table"><summary>Data table</summary>
<table>
<thead><tr><th>Measurement</th><th>Server</th><th>Median RSS (MiB)</th><th>Relative</th><th>Details</th></tr></thead>
<tbody>
<tr><td>Initialized</td><td>arity</td><td>25.900</td><td>baseline</td><td>PSS: 24.1 MiB; processes: 1</td></tr>
<tr><td>Initialized</td><td>languageserver</td><td>1051.000</td><td>40.58× arity</td><td>PSS: 899.3 MiB; processes: 9</td></tr>
<tr><td>Initialized</td><td>ark</td><td>119.300</td><td>4.61× arity</td><td>PSS: 111.0 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>arity</td><td>318.300</td><td>baseline</td><td>PSS: 316.5 MiB; processes: 1</td></tr>
<tr><td>Files open</td><td>languageserver</td><td>1772.800</td><td>5.57× arity</td><td>PSS: 1526.8 MiB; processes: 14</td></tr>
<tr><td>Files open</td><td>ark</td><td>124.200</td><td>0.39× arity</td><td>PSS: 116.0 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>arity</td><td>341.600</td><td>baseline</td><td>PSS: 339.7 MiB; processes: 1</td></tr>
<tr><td>After edits</td><td>languageserver</td><td>867.700</td><td>2.54× arity</td><td>PSS: 823.6 MiB; processes: 3</td></tr>
<tr><td>After edits</td><td>ark</td><td>132.900</td><td>0.39× arity</td><td>PSS: 124.3 MiB; processes: 1</td></tr>
<tr><td>Sampled peak</td><td>arity</td><td>361.600</td><td>baseline</td><td>PSS: 359.8 MiB; processes: 2</td></tr>
<tr><td>Sampled peak</td><td>languageserver</td><td>2025.300</td><td>5.60× arity</td><td>PSS: 1779.4 MiB; processes: 15</td></tr>
<tr><td>Sampled peak</td><td>ark</td><td>132.900</td><td>0.37× arity</td><td>PSS: 124.3 MiB; processes: 2</td></tr>
</tbody>
</table>
</details>
</div>
