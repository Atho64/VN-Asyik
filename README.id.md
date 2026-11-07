# VN Asyik

Pustaka visual novel dan pelacak waktu baca untuk Windows dan Android. Data
diambil dari VNDB, disimpan lokal di SQLite, dan tetap jalan tanpa jaringan.
Rust dan Tauri di sisi belakang, React di sisi depan.

[English](README.md) · **Bahasa Indonesia**

![Kisi pustaka, menampilkan gim yang sudah diimpor beserta kover dan waktu bacanya](src/assets/guide/library.jpg)

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

**Jurnal baca.** Kartu **Terakhir sampai mana?** di halaman gim menampilkan sesi
baca terakhir dan posisi bab/rute yang tercatat. Simpan catatan per sesi serta
penanda save slot manual, edit atau hapus entri, serta akses pintasan ke panduan.
Buka sesi mana pun di riwayat untuk melihat waktu, rute/bab yang tercatat, dan
semua penandanya, atau menambahkan catatan ke sesi lama tersebut. Tombol sesi
lebih lama/baru membuka seluruh riwayat per 50 sesi, dengan cuplikan catatan pada setiap sesi.
Jurnal ikut backup dan sinkronisasi pustaka. Save slot hanya pengingat; aplikasi
tidak memuat atau mengubah save game.

**Big Picture.** Peluncur layar penuh dengan judul dan artwork gim terpilih serta
deretan kover yang ringkas. Klik kover untuk memilih, lalu Mainkan atau Enter
untuk meluncurkan. Keyboard dan stik menjangkau seluruh koleksi; hanya kelompok
kover yang terlihat yang dirender. Buka dari library atau dengan Ctrl+Shift+B.

**Rute dan ending.** Rute sepenuhnya milikmu: kamu yang memberi nama, dan ending
menggantung padanya. Deteksi bab membaca judul jendela game saat engine-nya
menuliskannya (lihat [Bab dari judul jendela](#bab-dari-judul-jendela)), dan
jatuh ke atribusi tingkat gim saat tidak ada, jawaban yang jujur untuk gim yang
tidak melaporkan apa pun.

**Statistik.** Waktu baca per gim, per rute, dan per bulan, dihitung dari
sesinya sendiri.

**Discord Rich Presence.** Opsional, mati secara bawaan. Menampilkan apa yang
sedang kamu baca, bergiliran sesuai kolom yang kamu atur. Kover eksplisit
diblur dulu sebelum dikirim ke mana pun, kalau kamu memang mengaktifkan host
gambar.

**Panduan rute.** Ambil panduan pilihan rute langsung dari indeks
Seiya-Saiga. Langkah pilihan, rute, dan lencana ending disajikan rapi per
tab, lengkap dengan pencarian judul Jepang yang dinormalisasi dan opsi
terjemahan.

**Rekomendasi bacaan.** Tanya apa yang layak dibaca berikutnya. Profil selera
dihitung lokal dari tag VNDB gim-gim di pustakamu, dan pertanyaan bisa
menjangkau web terbuka lewat pencarian Tavily yang selalu menyertakan tautan
sumber halamannya, bukan tebakan tanpa rujukan.

**Sinkronisasi dan cadangan.** Selaraskan pustaka dan sesi baca antarperangkat
lewat folder bersama di jaringan lokal (LAN) tanpa bergantung pada server pihak
ketiga. Pustaka lengkap juga bisa diekspor ke satu berkas `.vn-asyik-library`
untuk dipindahkan atau dipulihkan kapan saja dengan penanganan konflik yang
transparan.

**Akses cepat.** Tekan `Ctrl+K` (atau `Cmd+K`) kapan saja untuk membuka palet
perintah: cari judul di pustaka seketika, pindah layar, atau kendalikan pelacakan
tanpa menyentuh mouse.

**Latar belakang & System Tray.** Di desktop, jendela VN Asyik dapat disembunyikan
ke taskbar notification area (system tray) lewat tombol samping, pintasan
`Ctrl+Shift+H`, Command Palette, atau dengan menutup jendela ('X'). Pelacakan
waktu bermain dan text hook LunaTranslator tetap berjalan di latar belakang tanpa
mengganggu visual novel yang sedang dibaca, dan jendela dapat dipanggil kembali
dengan sekali klik pada ikon tray.

**Android.** Waktu baca diambil dari event foreground `UsageStats`, yang butuh
izin Usage Access, baik untuk gim langsung maupun yang dijalankan lewat
emulator. Android tidak bisa melihat isi jendela aplikasi lain, jadi bab di
sana diatur manual dan aplikasi mengatakannya terang-terangan alih-alih
menebak.

Selama gim tetap di depan, pemeriksaan berkala memperbarui satu checkpoint lokal.
Riwayat mendapat satu sesi selesai ketika rentang baca berakhir, misalnya saat
berpindah aplikasi, mengunci layar, atau menekan Stop. Potongan riwayat 15 detik
yang sudah tersimpan tetap dipertahankan; perbaikan berlaku untuk rekaman baru.

## Bab dari judul jendela

Di desktop, bab diambil dari apa yang ditulis game di judul jendelanya sendiri:
tanpa injeksi, tanpa membaca memori, dan tidak rusak saat game di-update. Siglus
menaruh progresnya di sana, dan bentuk-bentuk ini sudah terverifikasi:

```text
Summer Pockets REFLECTION BLUE 静久 ８月７日   rute 静久, bab 8月7日
anemoi Ｄａｙ １                              bab Day 1
anemoi アネモイ Ｃｈａｐｔｅｒ ０１            rute アネモイ, bab Chapter 01
```

Karakter full-width dilipat dulu sebelum dicocokkan, jadi `Ｄａｙ １` dan
`Day 1` dibaca sama. Judul tanpa penanda (layar judul, menu) tidak
menghasilkan apa pun daripada mengarang bab; angka telanjang bukan bab; dan
judul yang tidak menyebut nama game yang sedang dilacak diabaikan.

### Pola khusus

Kalau game-mu menulis bentuk yang tidak tercakup di atas, buka **Pengaturan →
Umum → Membaca bab dari judul jendela**, ubah **Sumber** ke **Pola khusus**,
lalu isi polanya. Sintaksnya Rust [`regex`](https://docs.rs/regex) 1.x: waktu
linear, tanpa look-around, tanpa backreference, maksimal 512 byte (sekitar 170
karakter Jepang), dan dijalankan pada seluruh judul apa adanya, termasuk nama
game.

- **Dua grup:** yang pertama rute, yang kedua bab.
- **Satu grup:** itu babnya.

Minimal harus ada satu grup; pola tanpa grup ditolak saat disimpan. Grup yang
tidak ikut dalam kecocokan dilewati, jadi grup rute opsional menyisakan babnya
saja.

Dua sifat dialek itu yang perlu diketahui untuk caption Jepang: `\d` cocok
dengan angka Unicode apa pun (termasuk `１２` full-width), dan `(?i)` membuat
pencocokan tidak peka huruf besar/kecil.

Untuk judul seperti `Fate/stay night [Fate] Day 3`:

```text
^.*?\[(.+)\]\s*(.+)$    rute "Fate", bab "Day 3"
^.*?\s(Day \d+)$        bab "Day 3"                 (satu grup)
```

Yang perlu diketahui sebelum menyimpan:

- Pola tidak di-anchor: pola itu cocok di mana pun dalam judul kecuali kamu
  menulis `^` dan `$` sendiri.
- Kecocokan yang grupnya menangkap string kosong dianggap tidak cocok, nama bab
  kosong lebih buruk daripada tidak ada bab.
- Pola khusus **tidak** melipat karakter full-width: `Day` tidak pernah cocok
  dengan `Ｄａｙ`, karena keduanya karakter berbeda. Untuk caption yang ditulis
  begitu, tulis karakter tersebut di polanya, atau biarkan sumbernya di
  **Siglus** yang melipatnya.
- **Uji** menjalankan pola pada judul contoh, yang sudah terisi dari jendela
  yang ada di depan saat panel dibuka; ubah isinya untuk mencoba judul lain.
  Uji dulu sebelum menyimpan: pola yang tidak pernah cocok terlihat persis
  seperti game yang tidak menulis bab, dan tidak ada bagian lain di app yang
  bisa membedakannya.

**Mati** berhenti membaca judul sama sekali. Pilihannya disimpan lokal di
perangkat ini. Android tidak pernah membaca judul jendela (OS tidak
mengizinkannya), jadi bab di sana diisi manual.

## Fitur Beta

Teman baca, Temukan (pencarian VN dengan kunci API), dan Chat VN tersembunyi secara
bawaan. Aktifkan **Pengaturan → Umum → Fitur Beta** untuk menampilkan fitur dan
alat pengaturannya. Pilihan ini berlaku lokal per perangkat; impor pustaka tidak
mengaktifkannya. Menonaktifkannya menjeda teman baca, menghentikan tangkapan
jendela dan jawaban chat, memutus teks langsung Luna, serta menyembunyikan overlay.
Kunci, pengaturan, memori, dan riwayat tersimpan tetap ada. Mengaktifkannya kembali
tidak otomatis melanjutkan sesi atau tangkapan jendela. Pengembangan teman baca
untuk sementara dijeda. Pencarian pustaka, pencarian/impor VNDB, dan pelacakan
tetap tersedia.

## Obrolan pendamping

Pilih **Baca bersama teman** di halaman game, atau buka **Teman baca**, untuk
memulai sesi. Pilih game dan kepribadian, lalu gunakan teks yang
ditempel atau sumber LunaTranslator yang sudah kamu cocokkan dengan game itu.
Buka **Konteks bacaan** untuk mengatur nama sesi dan batas bacaan tambahan.
Jawaban tetap berjalan saat kamu berpindah halaman atau meminimalkan aplikasi
Windows. Jeda menghentikan dialog baru dan jawaban; **Hentikan jawaban** memutus
jawaban saat ini; **Akhiri sesi** menutup konteks. Setelah memuat save sebelumnya
atau berpindah rute, buka **Opsi sesi** (ikon roda gigi) lalu pilih **Sesi baru**.
Detail model, riwayat, dan Akhiri sesi ada di panel itu; Jeda dan Kirim tetap
di samping percakapan. Kirim menjadi Hentikan jawaban selama jawaban berjalan.
Detail tool yang selesai tersedia di bagian Sumber.

Batas pengetahuan dipilih secara jelas: **Metadata saja**, **Hanya yang kubagikan**,
atau **Semua spoiler**. Hanya baris Luna baru yang masih segar
setelah mulai atau lanjut yang dapat dipakai. Koneksi yang terputus atau
berganti menjeda sesi dan perlu dikonfirmasi sebelum dilanjutkan. Pertanyaan
menggunakan penyedia yang dipilih; reaksi otomatis dan saran rangkuman perlu
diaktifkan secara terpisah. Sesi
disimpan lokal, tidak ikut sinkronisasi atau ekspor pustaka, dan menyimpan
maksimal 200 pesan terbaru per percakapan. Riwayat yang sudah berakhir tetap
bisa dibaca atau dihapus setelah game dihapus. Memulai ulang aplikasi tidak
otomatis melanjutkan sesi.

Di desktop, **Buka overlay** membuka jendela kecil yang bisa dipindah, diubah
ukurannya, dan tetap di atas, dengan percakapan serta kontrol jeda/akhiri yang
sama. **Sembunyikan** atau tombol tutup menyembunyikannya sementara jawaban tetap
berjalan; buka lagi untuk mengikuti kelanjutannya. Pengaturan sesi, pengaturan
aplikasi, dan tautan pustaka dibuka di jendela utama. Gunakan mode berjendela atau
borderless: fullscreen eksklusif dapat menutupi jendela biasa. Android belum
memiliki overlay mengambang.

Pilih dari dua belas kepribadian, atau gunakan **Sesuaikan** untuk menyimpan
profil lokal dengan gaya bicara, minat, panjang balasan, bahasa, dan nadanya
sendiri. Contoh balasan berupa tulisan contoh; membukanya tidak memanggil model.
Di **Opsi sesi**, **Terapkan kepribadian** mengubah sesi saat ini dan menghentikan
balasan yang masih memakai kepribadian sebelumnya. Menyimpan atau menghapus profil
saja tidak mengubah salinan dalam sesi. Profil dan riwayat teman baca tidak ikut
ekspor koleksi maupun sinkronisasi.

**Opsi sesi → Tinjau ingatan** menambahkan ingatan lokal berkelanjutan (20C).
Gunakan **Ingat ini** untuk preferensi atau kutipan yang ditinjau, lalu pilih
cakupannya: semua pendamping, identitas ini, atau game dan alur pembacaan ini.
**Gunakan ingatan tersimpan** adalah pilihan terpisah yang awalnya nonaktif.
Ingatan dapat dicari, ditinjau, diedit, dan disematkan. **Sarankan rangkuman**
memanggil model terpilih saat diminta; saran rangkuman otomatis memerlukan
persetujuan terpisah dan mungkin dikenai biaya penyedia. Saran baru digunakan
setelah disetujui. Rangkuman terhenti menunggu percobaan ulang eksplisit, dan
bukti tersimpan tetap ada setelah riwayat melewati batas 200 pesan.

**Lupakan** menghapus teks ingatan dan bukti terpilih. Koreksi dan pelupaan juga
menghapus rangkuman turunan serta mengecualikan seluruh percakapan pendamping
sebelumnya dari prompt berikutnya, tetapi riwayat lokal tetap dapat dibaca.
Kesinambungan percakapan terbaru akan hilang; konten yang sudah dikirim ke
penyedia tidak dapat ditarik kembali. Sesi baru tidak mewarisi ingatan cerita
dari alur pembacaan lain. Ingatan tidak ikut sinkronisasi, cadangan, atau ekspor.
Menghapus sesi menghapus ingatan turunannya; preferensi yang ditulis mandiri
tetap tersimpan.

**Penglihatan** menambahkan adegan dari satu jendela terpilih di Windows (20D).
Pilih jendela game, periksa pratinjau lokal dan potongan opsional, lalu uji dukungan
gambar pada penyedia yang dipilih. Kirim satu gambar dengan pertanyaan, atau
aktifkan pengamatan dan reaksi otomatis sesekali saat game berada di depan.
Panggilan gambar dapat dikenai biaya. Panggilan otomatis berjarak minimal 60 detik,
maksimal 12/jam dan 30/sesi; panggilan yang diam, gagal, atau dibatalkan setelah
dikirim tetap dihitung. **Diam** menghentikan balasan otomatis; berhenti mengamati,
Jeda/Akhiri sesi, dan Lupakan membersihkan tangkapan. Pengamatan tidak dimulai ulang
otomatis saat aplikasi dibuka. Gambar hanya berada di memori; riwayat menyimpan
teks berlabel inferensi visual dan metadata sumber minimal. Tangkapan SDR Windows
didukung; HDR dan platform lain menampilkan status tidak tersedia.

Ini mencakup sesi teks (20A), identitas teman (20B), ingatan berkelanjutan (20C),
serta penglihatan dan reaksi sesekali (20D). Suara masih direncanakan. Android tidak
menjamin proses tetap berjalan di latar belakang; biarkan aplikasi terbuka di sana.

Obrolan ini bukan asisten umum yang ditempel ke pustaka. Ia bisa membaca dua hal
dan menyebut sumbernya:

- **Teks langsung.** Teks yang diekstrak LunaTranslator dari gim yang sedang
  kamu mainkan.
- **Skrip hasil impor.** Masukkan skrip yang kamu ekstrak sendiri: teks biasa,
  array JSON nama/pesan, atau format JSON Asyik. Aplikasi merangkumnya
  jadi digest cerita yang bisa dijadikan rujukan jawaban.

Perangkuman berjalan sebagai map-reduce atas skrip yang dipecah per anggaran
byte. Sampai 10 agen merangkum segmen berurutan secara paralel, masing-masing
membawa ekor ringkasan sebelumnya sebagai konteks, lalu digest tiap segmen
digabung jadi satu. Kemajuannya disimpan per potongan, jadi menutup aplikasi di
tengah jalan lalu kembali lagi akan melanjutkan, bukan mengulang dari nol.

Digest ditahan kecuali tingkat spoiler obrolan disetel ke _All_. Chat biasa
dapat membaca teks langsung pada semua tingkat spoiler; sesi teman baca juga
menahan dialog tangkapan dalam mode **Metadata saja**.

### Penyedia

Pengaturan menyimpan daftar penyedia, bukan satu endpoint saja: masing-masing
berisi nama, base URL yang kompatibel dengan OpenAI, dan model. Kuncinya tinggal
di credential store sistem, satu slot per penyedia, tidak pernah di database atau
file konfigurasi, dan berpindah penyedia tidak berarti mengetik ulang kunci.
Setelan tunggal yang ada sebelum daftar ini otomatis jadi satu profil saat
pertama dijalankan, lengkap dengan kuncinya.

Dua endpoint yang sudah terbukti jalan lewat lapisan kompatibilitas:

| Penyedia         | Endpoint                                                  | Contoh model       |
| ---------------- | --------------------------------------------------------- | ------------------ |
| Google AI Studio | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-2.5-flash` |
| Runtime lokal    | `http://127.0.0.1:11434/v1` (Ollama)                      | `qwen3`            |

`http` biasa hanya diterima di localhost, karena di tempat lain kuncinya akan
terkirim tanpa enkripsi.

## Yang tidak akan dilakukan

Sebagian karena batas platform, sebagian karena memang diputuskan begitu; tapi
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

Unduh prarilis terbaru dari [GitHub Releases](https://github.com/Atho64/VN-Asyik/releases).
Windows menyediakan installer `.exe` dan `.msi`; keduanya belum ditandatangani
dan mungkin menampilkan peringatan SmartScreen. Android menggunakan
`app-universal-release.apk` yang ditandatangani.

Untuk membangun dari kode sumber:

```
pnpm install --frozen-lockfile
pnpm tauri build
```

Hasilnya ada di `target/release/bundle/` kecuali direktori target Cargo diatur
sendiri. Android butuh JDK 21, Android SDK dan NDK, serta target Rust Android:

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

Mulai dari `crates/vn-core`: di situ logika domain murninya (state machine sesi,
penggabungan sinkronisasi, pemetaan bab, perangkuman skrip), tanpa Tauri dan
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

**[VaternID/EwE](https://github.com/VaternID/EwE)** (GPL-3.0, diport).
Kartu presence Discord dimodelkan dari EwE. `crates/vn-core/src/presence.rs`
mencatat apa yang diambil: pipeline transformasi teks per kolom (hapus, ganti,
awalan/akhiran, rapikan, dalam urutan itu) dan kursor rotasi terpisah per kolom.
Yang sengaja tidak diambil juga dicatat: skrip variabel Python kustom EwE, yang
menjalankan kode sembarang dari folder di sebelah executable, dan pengunggahan
kover tanpa persetujuan tersendiri.

**[dead8309/Kizzy](https://github.com/dead8309/Kizzy)** (GPL-3.0, inspirasi).
Rujukan untuk bicara ke Discord di Android, tempat soket IPC desktop tidak ada.
Jalur presence Android langsung terhubung ke Discord Gateway, pendekatan yang
ditunjukkan Kizzy. Tidak ada kode yang dipakai bersama.

**[Yukin0a/KireiBox](https://github.com/Yukin0a/KireiBox)** (GPL-3.0, inspirasi).
Rujukan bagaimana peluncur VN Android menyikapi mesin yang terpasang terpisah.
README-nya menarik garis antara penyerta mesin bawaan dan meluncurkan aplikasi
yang dipasang sendiri; VN Asyik mengambil jalur kedua dan tidak membundel apa pun.

**[LunaTranslator](https://github.com/HIllya51/LunaTranslator)** (GPL-3.0, terintegrasi).
Tersambung lewat layanan jaringannya yang terdokumentasi: endpoint websocket
mengalirkan teks hasil ekstraksi sebagai frame teks biasa, yang dikonsumsi
aplikasi ini. Tidak ada kode yang disalin; integrasinya adalah klien untuk API
yang terdokumentasi.

**LunaBox** (AGPL-3.0, hanya inspirasi).
Dibaca sebagai karya yang mendahului untuk membayangkan fitur AI di atas pustaka
VN, dan untuk tahu mode kegagalan apa yang penting (spoiler, kesimpulan karangan).
Ide dan perilaku fungsional tidak bisa dihakciptakan, jadi ini tidak menimbulkan
kewajiban apa pun.

## Lisensi

GPL-3.0-only. Lihat [LICENSE](LICENSE).

Font Inter Tight memakai SIL Open Font License 1.1. Teks hak cipta dan lisensi
lengkap tersedia di [README Inggris](README.md#inter-tight-license).
