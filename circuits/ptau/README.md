# Phase 1: powers of tau

The circuit uses the [Perpetual Powers of Tau](https://github.com/privacy-scaling-explorations/perpetualpowersoftau) from Privacy & Scaling Explorations, after 80 contributions, truncated to `2^14`. The circuit has 10,175 constraints, so `2^14 = 16,384` is enough.

The file is not committed. Download it here:

```sh
curl -o ppot_0080_14.ptau \
  https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_14.ptau
shasum -a 256 ppot_0080_14.ptau
# 3ca1149e9349b22b0ee0649399cfb787677129b7b1189d1899fc0d615d9583db
```

`snarkjs powersoftau verify ppot_0080_14.ptau` checks the whole chain of contributions in the file and ends with `Powers of Tau Ok!`. It takes several minutes.

The older Hermez download location no longer responds.
