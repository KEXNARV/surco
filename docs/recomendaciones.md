# Recomendaciones "según mis gustos"

Investigación del 2026-10-06. **[probado]** = llamada real desde esta máquina
(yt-dlp 2026.08.19); lo demás sale de la documentación citada.

## Fuentes de candidatos

| fuente | clave | costo | notas |
|---|---|---|---|
| Radio YT Music `RDAMVM<id>` vía yt-dlp | no | 1.55 s / 25 **[probado]** | videoIds directos: no hay que casar nada. Siempre con `-I 1:N`: sin límite tardó 12.6 s y dio 538 entradas, 321 únicas |
| Mix YouTube `RD<id>` | no | 1.06 s / 25 **[probado]** | mezcla videoclips; la de YT Music sale más "musical" |
| `music.youtube.com/youtubei/v1/next` directo | no | 0.93 s / 50 **[probado]** | API interna (lo que usa ytmusicapi); más frágil que yt-dlp |
| Deezer `/artist/{id}/related` y `/radio` | no | — **[probado]** | muy buena para rock en español; no existe radio por pista (error 600); ~50 req/5 s (terceros) |
| ListenBrainz Labs `similar-artists` | no | — **[probado]** | pide MBID de artista (MusicBrainz, 1 req/s y User-Agent obligatorio) |
| ListenBrainz LB Radio | token gratis | — | desde 2026 responde 401 sin token **[probado]** |
| Last.fm `track.getSimilar` | API key gratis | — | no se probó (no hay clave); los ToS exigen caché y no dan cifra de límite |
| AcousticBrainz | — | — | muerto desde 2022, no cubre música nueva |
| Embeddings locales (Discogs-EffNet ONNX, CLAP) | no | — | viable en la RTX, pero hay que bajar el audio de cada candidato: sirve para ordenar lo ya escuchado, no para descubrir. Licencia CC BY-NC-SA |

## Casar artista+título con YouTube (fuentes que no dan videoId)

- `ytsearch3:` 1.33 s; búsqueda YT Music filtro "songs" 0.82 s **[probado]**, con
  pistas "- Topic", álbum y duración. El tercer resultado fue otra canción: hay
  que puntuar, no tomar el primero.
- Puntuar como spotDL: artista ≥70 %, título >60 %, duración `exp(-0.1·Δs)`.
- Caché permanente (artista, título) → videoId; resolver perezoso solo las 2–3
  siguientes, como el prefetch actual. YouTube no busca por ISRC.

## Cómo lo hacen otros

- Navidrome 0.60: cadena `deezer,lastfm,listenbrainz` con respaldo + agente local
  por reproducciones y ratings; AudioMuse-AI ordena por similitud sónica.
- Jellyfin: géneros + azar (criticado). Symfonium: radio en el cliente por
  artistas similares.
- Señales habituales: skip < 30 s negativo, escucha completa positiva,
  repeticiones refuerzan (con ruido según usuario), favoritos fuertes, recencia.

## Diseño propuesto

1. **Candidatos:** radio YT Music de la pista actual
   (`yt-dlp --flat-playlist -J -I 1:50`), pedida cuando quedan pocas en cola.
2. **Historial:** una fila por reproducción: videoId, artista, título, inicio,
   segundos escuchados, duración, skip (< 30 s o < 50 %), completa, favorito.
3. **Orden por reglas, sin ML:** fuera lo que tiene 2+ skips o sonó hace poco;
   arriba artistas con buena tasa de escucha completa y favoritos; máximo una
   pista por artista cada X; fuera live/lyrics/cover/karaoke salvo que la
   semilla lo sea.
4. **Semilla:** la pista actual, o en modo "según mis gustos" una elegida al
   azar ponderado entre favoritos y más escuchadas.

Fase 2: descubrir fuera de la burbuja de YouTube con Deezer related/radio o LB
similar-artists, casando con puntuación y caché. Fase 3: embeddings locales
solo para refinar el orden.

## Fuentes

- https://ytmusicapi.readthedocs.io/en/latest/reference/watch.html
- https://forum.endeavouros.com/t/any-way-to-download-the-contents-of-a-youtube-mix/71147?page=2
- https://www.last.fm/api/tos · https://www.last.fm/api/show/track.getSimilar
- https://listenbrainz.readthedocs.io/en/latest/users/api/misc.html · https://troi.readthedocs.io/en/latest/lb_radio.html · https://labs.api.listenbrainz.org/
- https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting
- https://developers.deezer.com/api
- https://acousticbrainz.org/
- https://essentia.upf.edu/models.html · https://github.com/NeptuneHub/AudioMuse-AI
- https://navidrome.org/docs/usage/integration/external-services/
- https://spotdl.readthedocs.io/en/dev/reference/utils/matching/
- https://arxiv.org/pdf/1711.05237 · https://arxiv.org/pdf/2409.07367
