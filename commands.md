solana config get

<!-- vezi cat va costa deploy -->
ls -lh target/deploy/ico.so
solana rent 300000 --url https://api.devnet.solana.com



<!-- verifica tranzactie -->

solana confirm \
  3NEoRevdALpN9KiNaYc6CPBZTgm2YgY1B3jvY3HXfc5kCk8xa5GhKVRVnvexp27oMKw8KK1ESoEENQLZwmhcPS4t \
  --url https://api.devnet.solana.com \
  --verbose


  <!-- nu mai trebuie sa specifici accounts in anchor -->

  https://www.anchor-lang.com/docs/updates/release-notes/0-30-0#account-resolution



  <!-- trimite la o adresa fara fonduri -->

  solana transfer 147W9RVSuSknG7MMSzyRA3wnBqForAR96LAdot1LAFVA 1 --allow-unfunded-recipient