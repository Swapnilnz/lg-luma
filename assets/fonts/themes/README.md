# Theme fonts

Latin subsets (U+0020 to U+017F and common punctuation) of the Google Fonts families the Home
themes use, frozen at weight 400 and, where the family has one, 700. Hinting and layout tables
other than `kern` are dropped. Built with fontTools from
[google/fonts](https://github.com/google/fonts) at commit
`6c1dbe27dcf7d817d4b62cec033ba77fa8e2a5ec`; each family's licence is in `licenses/`.

`pnpm assets` copies these files next to the Inter weights; the renderer loads a theme's files
only when that theme draws text. A theme whose font file is missing falls back to Inter.

| Family | Files | Licence | Source |
| --- | --- | --- | --- |
| Figtree | `theme-figtree-*.ttf` | OFL 1.1 | `ofl/figtree/Figtree[wght].ttf` |
| Archivo Black | `theme-archivo-black-*.ttf` | OFL 1.1 | `ofl/archivoblack/ArchivoBlack-Regular.ttf` |
| JetBrains Mono | `theme-jetbrains-mono-*.ttf` | OFL 1.1 | `ofl/jetbrainsmono/JetBrainsMono[wght].ttf` |
| Manrope | `theme-manrope-*.ttf` | OFL 1.1 | `ofl/manrope/Manrope[wght].ttf` |
| Archivo | `theme-archivo-*.ttf` | OFL 1.1 | `ofl/archivo/Archivo[wdth,wght].ttf` |
| Oswald | `theme-oswald-*.ttf` | OFL 1.1 | `ofl/oswald/Oswald[wght].ttf` |
| Fraunces | `theme-fraunces-*.ttf` | OFL 1.1 | `ofl/fraunces/Fraunces[SOFT,WONK,opsz,wght].ttf` |
| Federo | `theme-federo-*.ttf` | OFL 1.1 | `ofl/federo/Federo-Regular.ttf` |
| Cormorant Garamond | `theme-cormorant-garamond-*.ttf` | OFL 1.1 | `ofl/cormorantgaramond/CormorantGaramond[wght].ttf` |
| Anton | `theme-anton-*.ttf` | OFL 1.1 | `ofl/anton/Anton-Regular.ttf` |
| Barlow | `theme-barlow-*.ttf` | OFL 1.1 | `ofl/barlow/Barlow-Regular.ttf`, `ofl/barlow/Barlow-Bold.ttf` |
| League Spartan | `theme-league-spartan-*.ttf` | OFL 1.1 | `ofl/leaguespartan/LeagueSpartan[wght].ttf` |
| Bangers | `theme-bangers-*.ttf` | OFL 1.1 | `ofl/bangers/Bangers-Regular.ttf` |
| Righteous | `theme-righteous-*.ttf` | OFL 1.1 | `ofl/righteous/Righteous-Regular.ttf` |
| Rubik | `theme-rubik-*.ttf` | OFL 1.1 | `ofl/rubik/Rubik[wght].ttf` |
| Roboto | `theme-roboto-*.ttf` | OFL 1.1 | `ofl/roboto/Roboto[wdth,wght].ttf` |
| Cinzel | `theme-cinzel-*.ttf` | OFL 1.1 | `ofl/cinzel/Cinzel[wght].ttf` |
| Fredoka | `theme-fredoka-*.ttf` | OFL 1.1 | `ofl/fredoka/Fredoka[wdth,wght].ttf` |
| Nunito | `theme-nunito-*.ttf` | OFL 1.1 | `ofl/nunito/Nunito[wght].ttf` |
| IBM Plex Mono | `theme-ibm-plex-mono-*.ttf` | OFL 1.1 | `ofl/ibmplexmono/IBMPlexMono-Regular.ttf`, `ofl/ibmplexmono/IBMPlexMono-Bold.ttf` |
| VT323 | `theme-vt323-*.ttf` | OFL 1.1 | `ofl/vt323/VT323-Regular.ttf` |
| Limelight | `theme-limelight-*.ttf` | OFL 1.1 | `ofl/limelight/Limelight-Regular.ttf` |
| Poiret One | `theme-poiret-one-*.ttf` | OFL 1.1 | `ofl/poiretone/PoiretOne-Regular.ttf` |
| DM Serif Display | `theme-dm-serif-display-*.ttf` | OFL 1.1 | `ofl/dmserifdisplay/DMSerifDisplay-Regular.ttf` |
| Syne | `theme-syne-*.ttf` | OFL 1.1 | `ofl/syne/Syne[wght].ttf` |
| Barlow Condensed | `theme-barlow-condensed-*.ttf` | OFL 1.1 | `ofl/barlowcondensed/BarlowCondensed-Regular.ttf`, `ofl/barlowcondensed/BarlowCondensed-Bold.ttf` |
| Monoton | `theme-monoton-*.ttf` | OFL 1.1 | `ofl/monoton/Monoton-Regular.ttf` |
| Bodoni Moda | `theme-bodoni-moda-*.ttf` | OFL 1.1 | `ofl/bodonimoda/BodoniModa[opsz,wght].ttf` |
| Special Elite | `theme-special-elite-*.ttf` | Apache 2.0 | `apache/specialelite/SpecialElite-Regular.ttf` |
| Share Tech Mono | `theme-share-tech-mono-*.ttf` | OFL 1.1 | `ofl/sharetechmono/ShareTechMono-Regular.ttf` |
| Unbounded | `theme-unbounded-*.ttf` | OFL 1.1 | `ofl/unbounded/Unbounded[wght].ttf` |
| Courier Prime | `theme-courier-prime-*.ttf` | OFL 1.1 | `ofl/courierprime/CourierPrime-Regular.ttf`, `ofl/courierprime/CourierPrime-Bold.ttf` |
| Press Start 2P | `theme-press-start-2p-*.ttf` | OFL 1.1 | `ofl/pressstart2p/PressStart2P-Regular.ttf` |
| Audiowide | `theme-audiowide-*.ttf` | OFL 1.1 | `ofl/audiowide/Audiowide-Regular.ttf` |
| Orbitron | `theme-orbitron-*.ttf` | OFL 1.1 | `ofl/orbitron/Orbitron[wght].ttf` |
