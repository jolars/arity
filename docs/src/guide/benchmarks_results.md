### Formatter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":4.1507,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.6205,"min_ms":3.2474,"max_ms":9.5211},{"document":"MASS/polr.R","tool":"air","mean_ms":4.8156,"ratio":1.1601898474956032,"ratio_label":"1.2x slower","stddev_ms":0.3739,"min_ms":4.2152,"max_ms":6.3172},{"document":"MASS/polr.R","tool":"styler","mean_ms":3051.0443,"ratio":735.0674103163323,"ratio_label":"735.1x slower","stddev_ms":44.5017,"min_ms":2999.7314,"max_ms":3079.0771},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":4.0095,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.8169,"min_ms":2.7427,"max_ms":12.5569},{"document":"tidyr/pivot-wide.R","tool":"air","mean_ms":3.1668,"ratio":0.7898241676019453,"ratio_label":"1.3x faster","stddev_ms":0.3491,"min_ms":2.6023,"max_ms":4.8536},{"document":"tidyr/pivot-wide.R","tool":"styler","mean_ms":1706.3073,"ratio":425.56610549943883,"ratio_label":"425.6x slower","stddev_ms":10.4165,"min_ms":1696.1539,"max_ms":1716.9683},{"document":"small","tool":"arity","mean_ms":5.9151,"ratio":1.0,"ratio_label":"baseline","stddev_ms":0.8712,"min_ms":4.4577,"max_ms":12.38},{"document":"small","tool":"air","mean_ms":14.4698,"ratio":2.4462477388378896,"ratio_label":"2.4x slower","stddev_ms":0.934,"min_ms":12.504,"max_ms":16.3724},{"document":"small","tool":"styler","mean_ms":12798.6383,"ratio":2163.723064698822,"ratio_label":"2163.7x slower","stddev_ms":77.8491,"min_ms":12709.2134,"max_ms":12851.2799},{"document":"large","tool":"arity","mean_ms":42.0351,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.45,"min_ms":37.5381,"max_ms":48.7449},{"document":"large","tool":"air","mean_ms":158.7136,"ratio":3.77573979840657,"ratio_label":"3.8x slower","stddev_ms":39.5942,"min_ms":140.0943,"max_ms":264.1431}]</script>
<figcaption>Formatting speed on single files relative to arity, one dot per document: the largest source file of each benchmarked package, then two synthetic corpus tiers. The vertical axis is mean wall-clock time as a ratio to arity on a log scale, so arity lies on the dashed baseline at 1; faster tools fall below it and slower tools rise above. Hover a dot for the exact figures.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.1507</td><td>3.2474</td><td>9.5211</td><td>baseline</td></tr>
<tr><td>air</td><td>4.8156</td><td>4.2152</td><td>6.3172</td><td>1.2x slower</td></tr>
<tr><td>styler</td><td>3051.0443</td><td>2999.7314</td><td>3079.0771</td><td>735.1x slower</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>4.0095</td><td>2.7427</td><td>12.5569</td><td>baseline</td></tr>
<tr><td>air</td><td>3.1668</td><td>2.6023</td><td>4.8536</td><td>1.3x faster</td></tr>
<tr><td>styler</td><td>1706.3073</td><td>1696.1539</td><td>1716.9683</td><td>425.6x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>5.9151</td><td>4.4577</td><td>12.3800</td><td>baseline</td></tr>
<tr><td>air</td><td>14.4698</td><td>12.5040</td><td>16.3724</td><td>2.4x slower</td></tr>
<tr><td>styler</td><td>12798.6383</td><td>12709.2134</td><td>12851.2799</td><td>2163.7x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>42.0351</td><td>37.5381</td><td>48.7449</td><td>baseline</td></tr>
<tr><td>air</td><td>158.7136</td><td>140.0943</td><td>264.1431</td><td>3.8x slower</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":20.6405,"ratio":1.0,"ratio_label":"baseline","stddev_ms":2.2835,"min_ms":18.8363,"max_ms":32.0159},{"document":"tidyr","tool":"air","mean_ms":18.3195,"ratio":0.8875511736634288,"ratio_label":"1.1x faster","stddev_ms":0.7866,"min_ms":16.9617,"max_ms":20.9987},{"document":"MASS","tool":"arity","mean_ms":30.5878,"ratio":1.0,"ratio_label":"baseline","stddev_ms":1.1588,"min_ms":28.3946,"max_ms":34.1855},{"document":"MASS","tool":"air","mean_ms":31.9846,"ratio":1.0456652652364669,"ratio_label":"1.0x slower","stddev_ms":0.9949,"min_ms":30.4598,"max_ms":34.8159}]</script>
<figcaption>Formatting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>20.6405</td><td>18.8363</td><td>32.0159</td><td>baseline</td></tr>
<tr><td>air</td><td>18.3195</td><td>16.9617</td><td>20.9987</td><td>1.1x faster</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>30.5878</td><td>28.3946</td><td>34.1855</td><td>baseline</td></tr>
<tr><td>air</td><td>31.9846</td><td>30.4598</td><td>34.8159</td><td>1.0x slower</td></tr>
</tbody>
</table>
</details>
</div>

### Linter

#### Single files

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"MASS/polr.R","tool":"arity","mean_ms":2884.8522,"ratio":1.0,"ratio_label":"baseline","stddev_ms":103.1413,"min_ms":2766.299,"max_ms":2953.9761},{"document":"MASS/polr.R","tool":"jarl","mean_ms":29.4329,"ratio":0.010202567743331878,"ratio_label":"98.0x faster","stddev_ms":4.9082,"min_ms":21.7779,"max_ms":46.8345},{"document":"MASS/polr.R","tool":"lintr","mean_ms":2476.0777,"ratio":0.8583031394121334,"ratio_label":"1.2x faster","stddev_ms":13.7214,"min_ms":2460.314,"max_ms":2485.3402},{"document":"tidyr/pivot-wide.R","tool":"arity","mean_ms":344.6711,"ratio":1.0,"ratio_label":"baseline","stddev_ms":54.3525,"min_ms":300.299,"max_ms":451.3638},{"document":"tidyr/pivot-wide.R","tool":"jarl","mean_ms":16.6607,"ratio":0.048337966252465024,"ratio_label":"20.7x faster","stddev_ms":6.4393,"min_ms":9.5491,"max_ms":36.539},{"document":"tidyr/pivot-wide.R","tool":"lintr","mean_ms":906.9159,"ratio":2.6312501976521965,"ratio_label":"2.6x slower","stddev_ms":77.7114,"min_ms":860.4728,"max_ms":996.6306},{"document":"small","tool":"arity","mean_ms":114.4648,"ratio":1.0,"ratio_label":"baseline","stddev_ms":4.0685,"min_ms":110.2458,"max_ms":124.0183},{"document":"small","tool":"jarl","mean_ms":31.3489,"ratio":0.2738737148887693,"ratio_label":"3.7x faster","stddev_ms":1.4203,"min_ms":29.1067,"max_ms":36.38},{"document":"small","tool":"lintr","mean_ms":11577.9805,"ratio":101.14882915970674,"ratio_label":"101.1x slower","stddev_ms":113.3796,"min_ms":11491.1552,"max_ms":11706.2516},{"document":"large","tool":"arity","mean_ms":12873.2063,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3026.9824,"min_ms":10746.8662,"max_ms":16338.7982},{"document":"large","tool":"jarl","mean_ms":452.2557,"ratio":0.03513155071553541,"ratio_label":"28.5x faster","stddev_ms":4.7833,"min_ms":448.3683,"max_ms":461.0907}]</script>
<figcaption>Linting speed on single files relative to arity, one dot per document, on the same log-ratio axis as the formatter charts.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>MASS/polr.R (19787 bytes, 534 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>2884.8522</td><td>2766.2990</td><td>2953.9761</td><td>baseline</td></tr>
<tr><td>jarl</td><td>29.4329</td><td>21.7779</td><td>46.8345</td><td>98.0x faster</td></tr>
<tr><td>lintr</td><td>2476.0777</td><td>2460.3140</td><td>2485.3402</td><td>1.2x faster</td></tr>
</tbody>
</table>
<h5>tidyr/pivot-wide.R (23349 bytes, 807 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>344.6711</td><td>300.2990</td><td>451.3638</td><td>baseline</td></tr>
<tr><td>jarl</td><td>16.6607</td><td>9.5491</td><td>36.5390</td><td>20.7x faster</td></tr>
<tr><td>lintr</td><td>906.9159</td><td>860.4728</td><td>996.6306</td><td>2.6x slower</td></tr>
</tbody>
</table>
<h5>small (148330 bytes, 10170 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>114.4648</td><td>110.2458</td><td>124.0183</td><td>baseline</td></tr>
<tr><td>jarl</td><td>31.3489</td><td>29.1067</td><td>36.3800</td><td>3.7x faster</td></tr>
<tr><td>lintr</td><td>11577.9805</td><td>11491.1552</td><td>11706.2516</td><td>101.1x slower</td></tr>
</tbody>
</table>
<h5>large (1779960 bytes, 122040 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>12873.2063</td><td>10746.8662</td><td>16338.7982</td><td>baseline</td></tr>
<tr><td>jarl</td><td>452.2557</td><td>448.3683</td><td>461.0907</td><td>28.5x faster</td></tr>
</tbody>
</table>
</details>
</div>

#### Projects

<div class="bench-chart-block">
<figure class="bench-figure">
<div class="bench-chart"></div>
<script type="application/json" class="bench-data">[{"document":"tidyr","tool":"arity","mean_ms":165.9217,"ratio":1.0,"ratio_label":"baseline","stddev_ms":3.5125,"min_ms":160.6096,"max_ms":173.6833},{"document":"tidyr","tool":"jarl","mean_ms":13.4762,"ratio":0.08122023822079934,"ratio_label":"12.3x faster","stddev_ms":0.778,"min_ms":12.1676,"max_ms":16.4609},{"document":"tidyr","tool":"lintr","mean_ms":8777.2541,"ratio":52.89997691682282,"ratio_label":"52.9x slower","stddev_ms":399.6911,"min_ms":8446.6291,"max_ms":9221.4362},{"document":"MASS","tool":"arity","mean_ms":1372.6034,"ratio":1.0,"ratio_label":"baseline","stddev_ms":7.3714,"min_ms":1367.265,"max_ms":1381.014},{"document":"MASS","tool":"jarl","mean_ms":18.1169,"ratio":0.013198932772569266,"ratio_label":"75.8x faster","stddev_ms":1.1231,"min_ms":16.2523,"max_ms":22.4881},{"document":"MASS","tool":"lintr","mean_ms":9110.6801,"ratio":6.637518237241726,"ratio_label":"6.6x slower","stddev_ms":17.6525,"min_ms":9090.8465,"max_ms":9124.6693}]</script>
<figcaption>Linting speed on real R packages (the tidyr and MASS source trees) relative to arity, on the same log-ratio axis.</figcaption>
</figure>
<noscript>Enable JavaScript for the interactive chart; the data table below has the same numbers.</noscript>
<details class="bench-table">
<summary>Data table</summary>
<h5>tidyr (245685 bytes, 8774 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>165.9217</td><td>160.6096</td><td>173.6833</td><td>baseline</td></tr>
<tr><td>jarl</td><td>13.4762</td><td>12.1676</td><td>16.4609</td><td>12.3x faster</td></tr>
<tr><td>lintr</td><td>8777.2541</td><td>8446.6291</td><td>9221.4362</td><td>52.9x slower</td></tr>
</tbody>
</table>
<h5>MASS (214820 bytes, 5951 lines)</h5>
<table>
<thead><tr><th>Tool</th><th>Mean (ms)</th><th>Min (ms)</th><th>Max (ms)</th><th>Relative</th></tr></thead>
<tbody>
<tr><td>arity</td><td>1372.6034</td><td>1367.2650</td><td>1381.0140</td><td>baseline</td></tr>
<tr><td>jarl</td><td>18.1169</td><td>16.2523</td><td>22.4881</td><td>75.8x faster</td></tr>
<tr><td>lintr</td><td>9110.6801</td><td>9090.8465</td><td>9124.6693</td><td>6.6x slower</td></tr>
</tbody>
</table>
</details>
</div>

