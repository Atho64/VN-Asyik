# VN Asyik

Pustaka visual novel dan pelacak waktu baca untuk Windows dan Android. Data
diambil dari VNDB, disimpan lokal di SQLite, dan tetap jalan tanpa jaringan.
Rust dan Tauri di sisi belakang, React di sisi depan.

[English](README.md) · **Bahasa Indonesia**

![Kisi pustaka, menampilkan gim yang sudah diimpor beserta kover dan waktu bacanya](docs/screenshot-library.jpg)

## Yang bisa dilakukan

**Pustaka.** Cari di VNDB lalu impor gim lengkap dengan kover, tag, rating, dan
galeri tangkapan layarnya. Tampilan bisa kisi, daftar, atau rak; ketiganya
divirtualisasi supaya tetap ringan. Kover eksplisit tetap tertutup di balik
permukaan netral sampai kamu minta melihatnya, dan mengubah pengaturan itu
menutupnya kembali.

**Waktu baca.** Jalankan gim dari aplikasi dan waktu bacamu dicatat. Waktu hanya
dianggap berjalan selama jendela gim ada di depanmu; jeda yang melewati ambang
tertentu dicatat terpisah sebagai waktu pergi, bukan waktu baca, dan keduanya
bisa kamu lihat sendiri-sendiri. Sesi ditulis tiap 20 detik, jadi crash atau
reboot paksa kehilangan hitungan detik, bukan sepanjang malam.

**Rute dan ending.** Rute sepenuhnya milikmu: kamu yang memberi nama, dan ending
menggantung padanya. Deteksi bab membaca keadaan mesin saat ada adapter yang
sudah terverifikasi, dan jatuh ke atribusi tingkat gim saat tidak ada — jawaban
yang jujur untuk gim yang build-nya belum pernah dianalisis siapa pun.

**Statistik.** Waktu baca per gim, per rute, dan per bulan, dihitung dari
sesinya sendiri.

**Discord Rich Presence.** Opsional, mati secara bawaan. Menampilkan apa yang
sedang kamu baca, bergiliran sesuai kolom yang kamu atur. Kover eksplisit
diblur dulu sebelum dikirim ke mana pun, kalau kamu memang mengaktifkan host
gambar.

**Android.** Waktu baca diambil dari event foreground `UsageStats`, yang butuh
izin Usage Access. Android tidak bisa melihat isi jendela aplikasi lain, jadi
bab di sana diatur manual dan aplikasi mengatakannya terang-terangan alih-alih
menebak.

## Obrolan pendamping

Obrolan ini bukan asisten umum yang ditempel ke pustaka. Ia bisa membaca dua hal
dan menyebut sumbernya:

- **Teks langsung.** Teks yang diekstrak LunaTranslator dari gim yang sedang
  kamu mainkan.
- **Skrip hasil impor.** Masukkan skrip yang kamu ekstrak sendiri — teks biasa,
  array JSON nama/pesan, atau format JSON Asyik — lalu aplikasi merangkumnya
  jadi digest cerita yang bisa dijadikan rujukan jawaban.

Perangkuman berjalan sebagai map-reduce atas skrip yang dipecah per anggaran
byte. Sampai 10 agen merangkum segmen berurutan secara paralel, masing-masing
membawa ekor ringkasan sebelumnya sebagai konteks, lalu digest tiap segmen
digabung jadi satu. Kemajuannya disimpan per potongan, jadi menutup aplikasi di
tengah jalan lalu kembali lagi akan melanjutkan, bukan mengulang dari nol.

Digest ditahan kecuali tingkat spoiler obrolan disetel ke _All_. Teks langsung
tidak ditahan, karena kamu memang sudah membacanya.

## Yang tidak akan dilakukan

Sebagian karena batas platform, sebagian karena memang diputuskan begitu — tapi
apa pun alasannya, aplikasi mengatakannya, bukan berpura-pura.

- **Tidak ada deteksi bab di Android.** Sistem operasi tidak mengizinkan satu
  aplikasi memeriksa jendela aplikasi lain. Bab dan rute diatur manual.
- **Aplikasi tidak pernah menulis ke gim yang sedang berjalan.** Tidak menambal
  berkas, tidak menyuntik ulang teks terjemahan, tidak menyentuh DRM.
- **Tanpa telemetri.** Tidak ada analitik, tidak ada pelaporan crash, tidak ada
  laporan pulang ke siapa pun. Koneksi jaringan hanya yang kamu nyalakan sendiri:
  VNDB, Discord, penyedia obrolan pilihanmu, dan Tavily kalau kamu minta
  rekomendasi yang butuh internet terbuka.

## Pemasangan

Build dibuat sendiri di mesinmu; belum ada rilis publik yang ditandatangani.

```
pnpm install --frozen-lockfile
pnpm tauri build
```

Hasilnya ada di `src-tauri/target/release/bundle/`. Android butuh JDK 21,
Android SDK dan NDK, serta target Rust Android:

```
pnpm tauri android build --apk --target aarch64
```

Application ID-nya `com.atho.vnasyik`. Android mengikat data aplikasi, penyimpanan
kredensial Keystore, dan izin Usage Access ke string itu, jadi sifatnya permanen:
mengubahnya lagi membuat sistem menganggapnya aplikasi yang berbeda.

## Pindahan dari build `app.vntracker.desktop`

Paketnya pernah diganti nama sekali, sebelum rilis publik pertama. Build dari
sebelum penggantian tidak bisa memperbarui di tempat, karena Android melihatnya
sebagai aplikasi lain: ia terpasang di samping yang lama dengan pustaka kosong.

- **Android:** ekspor lewat _Pengaturan → Backup & transfer_ di build lama, lalu
  impor di build ini.
- **Windows:** pustakanya ada di `%APPDATA%\com.atho.vnasyik\`. Salin foldernya
  dari `%APPDATA%\app.vntracker.desktop\` sebelum menjalankan yang baru, dan
  gim, sesi, serta pengaturannya ikut pindah.

## Pengembangan

```
pnpm install --frozen-lockfile
pnpm tauri dev
```

```
pnpm lint && pnpm typecheck && pnpm test
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

Tes pelacakan langsung menyentuh jendela dan waktu diam yang sesungguhnya, jadi
dilewati secara bawaan:

```
cargo test --test tracking_live -- --ignored --nocapture
```

Pemeriksaan kompilasi Android, yang menangkap kode yang bocor keluar dari
gerbang `#[cfg]`:

```
cargo check --target aarch64-linux-android -p vn-tracker
```

## Membaca kodenya

Mulai dari `crates/vn-core`: di situ logika domain murninya — state machine sesi,
penggabungan sinkronisasi, pemetaan bab, perangkuman skrip — tanpa Tauri dan
tanpa panggilan OS, jadi bisa dites dengan `cargo test` biasa. `src-tauri/src`
membungkusnya jadi command, penyimpanan, dan adapter platform. `src/` adalah
frontend React yang hanya bicara ke backend lewat `src/lib/ipc`.

Komentar di kode menjelaskan _kenapa_ sebuah aturan ada, di tempat yang tidak
jelas dengan sendirinya, supaya alasannya ikut bersama kodenya alih-alih pindah
ke dokumen terpisah.

## Karya yang mendahului dan atribusi

Tiap butir di bawah menyebut apa yang benar-benar diberikannya ke proyek ini.
_Diport_ berarti kode atau konstanta disalin dan membawa komentar asal-usul di
sumbernya. _Inspirasi_ berarti proyeknya dibaca untuk memahami masalahnya; tidak
ada yang disalin, dan sebuah ide tidak menimbulkan kewajiban apa pun.

**[VaternID/EwE](https://github.com/VaternID/EwE) — GPL-3.0 — diport.**
Kartu presence Discord dimodelkan dari EwE. `crates/vn-core/src/presence.rs`
mencatat apa yang diambil: pipeline transformasi teks per kolom (hapus, ganti,
awalan/akhiran, rapikan, dalam urutan itu) dan kursor rotasi terpisah per kolom.
Yang sengaja tidak diambil juga dicatat — skrip variabel Python kustom EwE, yang
menjalankan kode sembarang dari folder di sebelah executable, dan pengunggahan
kover tanpa persetujuan tersendiri.

**[dead8309/Kizzy](https://github.com/dead8309/Kizzy) — GPL-3.0 — inspirasi.**
Rujukan untuk bicara ke Discord di Android, tempat soket IPC desktop tidak ada.
Jalur presence Android langsung terhubung ke Discord Gateway, pendekatan yang
ditunjukkan Kizzy. Tidak ada kode yang dipakai bersama.

**[Yukin0a/KireiBox](https://github.com/Yukin0a/KireiBox) — GPL-3.0 — inspirasi.**
Rujukan bagaimana peluncur VN Android menyikapi mesin yang terpasang terpisah.
README-nya menarik garis antara penyerta mesin bawaan dan meluncurkan aplikasi
yang dipasang sendiri; VN Asyik mengambil jalur kedua dan tidak membundel apa pun.

**[LunaTranslator](https://github.com/HIllya51/LunaTranslator) — GPL-3.0 — terintegrasi.**
Tersambung lewat layanan jaringannya yang terdokumentasi: endpoint websocket
mengalirkan teks hasil ekstraksi sebagai frame teks biasa, yang dikonsumsi
aplikasi ini. Tidak ada kode yang disalin; integrasinya adalah klien untuk API
yang terdokumentasi.

**LunaBox — AGPL-3.0 — hanya inspirasi.**
Dibaca sebagai karya yang mendahului untuk membayangkan fitur AI di atas pustaka
VN, dan untuk tahu mode kegagalan apa yang penting (spoiler, kesimpulan karangan).
Ide dan perilaku fungsional tidak bisa dihakciptakan, jadi ini tidak menimbulkan
kewajiban apa pun.

## Lisensi

GPL-3.0-only. Lihat [LICENSE](LICENSE).
