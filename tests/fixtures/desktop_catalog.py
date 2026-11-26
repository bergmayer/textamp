"""Deterministic catalog and read/write API for desktop parity checks.

Imported only by the loopback UI fixture. All data is synthetic; requests are
recorded without authentication parameters. No external service is contacted.
"""
import io
import math
import struct
import wave

ARTISTS = [{'id': f'artist-{i}', 'name': f'{chr(65+i)} Artist', 'albumCount': 2,
            'coverArt': f'artist-{i}'} for i in range(26)]
ALBUMS = [{'id': f'album-{a}-{b}', 'name': f'{chr(65+a)} Album {b+1}',
           'artist': ARTISTS[a]['name'], 'artistId': ARTISTS[a]['id'], 'songCount': 15,
           'duration': 900, 'year': 2000+a, 'genre': 'Fixture Rock', 'coverArt': f'album-{a}-{b}'}
          for a in range(26) for b in range(2)]
SONGS = [{'id': f'song-{a}-{b}-{t}', 'title': f'{chr(65+a)} Track {b+1}.{t+1:02}',
          'artist': ARTISTS[a]['name'], 'artistId': ARTISTS[a]['id'],
          'album': f'{chr(65+a)} Album {b+1}', 'albumId': f'album-{a}-{b}',
          'parent': f'album-{a}-{b}', 'track': t+1, 'duration': 60,
          'year': 2000+a, 'genre': 'Fixture Rock', 'coverArt': f'album-{a}-{b}',
          'size': 44 + 8000 * 60 * 2, 'contentType': 'audio/wav', 'suffix': 'wav', 'bitRate': 128,
          'isDir': False, 'path': f'{ARTISTS[a]["name"]}/Album {b+1}/Track {t+1:02}.wav'}
         for a in range(26) for b in range(2) for t in range(15)]
PLAYLISTS = [{'id': 'playlist-1', 'name': 'Fixture Playlist', 'songCount': 30,
              'duration': 1800, 'public': False, 'owner': 'fixture'},
             {'id': 'playlist-2', 'name': 'Second Fixture Playlist', 'songCount': 30,
              'duration': 1800, 'public': False, 'owner': 'fixture'}]
EVENTS = []
SAVED_PLAYLIST_TRACKS = {}

def populate(artist_count):
    """Generate a bounded catalog for local responsiveness measurements."""
    if not 1 <= artist_count <= 25000:
        raise ValueError('artist count must be between 1 and 25000')
    global ARTISTS, ALBUMS, SONGS
    ARTISTS = [{'id': f'artist-{i}', 'name': f'Artist {i:05}', 'albumCount': 2,
                'coverArt': f'artist-{i}'} for i in range(artist_count)]
    ALBUMS = [{'id': f'album-{a}-{b}', 'name': f'Album {a:05}.{b}',
               'artist': ARTISTS[a]['name'], 'artistId': ARTISTS[a]['id'], 'songCount': 5,
               'duration': 300, 'year': 2000, 'coverArt': f'album-{a}-{b}'}
              for a in range(artist_count) for b in range(2)]
    SONGS = [{'id': f'song-{a}-{b}-{t}', 'title': f'Track {a:05}.{b}.{t}',
              'artist': ARTISTS[a]['name'], 'artistId': ARTISTS[a]['id'],
              'album': f'Album {a:05}.{b}', 'albumId': f'album-{a}-{b}',
              'parent': f'album-{a}-{b}', 'track': t+1, 'duration': 60,
              'year': 2000, 'coverArt': f'album-{a}-{b}',
              'size': 44 + 8000 * 60 * 2, 'contentType': 'audio/wav', 'suffix': 'wav', 'isDir': False,
              'path': f'Artist {a:05}/Album {b}/Track {t}.wav'}
             for a in range(artist_count) for b in range(2) for t in range(5)]

def one(rows, key):
    return next((dict(row) for row in rows if row['id'] == key), {})

def response(endpoint, query):
    value = lambda key, default='': query.get(key, [default])[0]
    key = value('id')
    if endpoint == 'getArtists': return {'artists': {'index': [{'name': 'Fixture', 'artist': ARTISTS}]}}
    if endpoint == 'getArtist':
        artist = one(ARTISTS, key)
        artist['album'] = [a for a in ALBUMS if a['artistId'] == key]
        return {'artist': artist}
    if endpoint == 'getAlbum':
        album = one(ALBUMS, key)
        album['song'] = [s for s in SONGS if s['albumId'] == key]
        return {'album': album}
    if endpoint == 'getSong': return {'song': one(SONGS, key)}
    if endpoint == 'getAlbumList2':
        offset, size = int(value('offset', '0')), int(value('size', '500'))
        return {'albumList2': {'album': ALBUMS[offset:offset+size]}}
    if endpoint == 'getGenres': return {'genres': {'genre': [{'value': 'Fixture Rock', 'songCount': len(SONGS), 'albumCount': len(ALBUMS)}]}}
    if endpoint == 'getSongsByGenre': return {'songsByGenre': {'song': SONGS[:int(value('count', '50'))]}}
    if endpoint == 'getRandomSongs': return {'randomSongs': {'song': SONGS[:int(value('size', '25'))]}}
    if endpoint in ('getSimilarSongs', 'getSimilarSongs2'): return {endpoint[3:4].lower()+endpoint[4:]: {'song': SONGS[1:8] + SONGS[30:45] + SONGS[60:75]}}
    if endpoint == 'getAlbumInfo2':
        if key == 'album-1-0':
            info = {'lastFmUrl': 'https://music.apple.com/us/album/fixture/456'}
            reads = sum(event['endpoint'] == endpoint and event['ids'] == [key] for event in EVENTS)
            if reads % 2 == 0:
                info['notes'] = '<p>Fixture album description.</p><p>Source: Wikipedia</p>'
            return {'albumInfo': info}
        if key == 'album-0-1':
            return {'albumInfo': {'lastFmUrl': 'https://music.apple.com/us/album/fixture/123'}}
        return {'albumInfo': {'notes': '<p>Fixture <i>review</i> for '+key+'</p><p>Source: Apple Music</p>'}}
    if endpoint == 'getArtistInfo2': return {'artistInfo2': {'biography': 'A synthetic artist biography for native UI testing.', 'similarArtist': ARTISTS[1:4]}}
    if endpoint == 'getTopSongs': return {'topSongs': {'song': SONGS[:15]}}
    if endpoint == 'getStarred2': return {'starred2': {'artist': ARTISTS[:2], 'album': ALBUMS[:2], 'song': SONGS[:4]}}
    if endpoint == 'getPlaylists': return {'playlists': {'playlist': PLAYLISTS}}
    if endpoint == 'getPlaylist':
        playlist = one(PLAYLISTS, key)
        playlist['entry'] = SAVED_PLAYLIST_TRACKS.get(key, SONGS[30:60] if key == 'playlist-2' else SONGS[:30])
        return {'playlist': playlist}
    if endpoint == 'search3':
        text = value('query').lower().replace('*', '')
        def page(rows, field, title):
            rows = [r for r in rows if text in r[title].lower()]
            offset = int(value(field+'Offset', '0'))
            count = int(value(field+'Count', '500'))
            return rows[offset:offset+count]
        return {'searchResult3': {'artist': page(ARTISTS, 'artist', 'name'),
                                 'album': page(ALBUMS, 'album', 'name'),
                                 'song': page(SONGS, 'song', 'title')}}
    if endpoint == 'getLyrics': return {'lyrics': {'artist': 'Fixture', 'title': 'Fixture Song', 'value': 'Synthetic fixture lyrics.\nSecond fixture line.'}}
    if endpoint == 'getLyricsBySongId' and key == 'song-1-0-0':
        lines = [{'start': i * 2000, 'value': f'Fixture lyric {i + 1}'} for i in range(30)]
        return {'lyricsList': {'structuredLyrics': [{'lang': 'en', 'synced': True, 'line': lines}]}}
    if endpoint == 'getLyricsBySongId': return {'lyricsList': {'structuredLyrics': [
        {'lang': 'en', 'synced': True, 'line': [{'start': 0, 'value': 'Synthetic fixture lyrics.'}, {'start': 30000, 'value': 'Second fixture line.'}]},
        {'lang': 'fr', 'synced': False, 'line': [{'value': 'Paroles de test.'}]}]}}
    if endpoint == 'getIndexes': return {'indexes': {'index': [{'name': 'Fixture', 'artist': ARTISTS}]}}
    if endpoint == 'getMusicDirectory':
        if key.startswith('artist-'): children = [dict(a, title=a['name'], isDir=True) for a in ALBUMS if a['artistId'] == key]
        elif key.startswith('album-'): children = [s for s in SONGS if s['albumId'] == key]
        else: children = [dict(a, title=a['name'], isDir=True) for a in ARTISTS]
        return {'directory': {'id': key, 'name': key or 'Fixture', 'child': children}}
    if endpoint == 'createPlaylist':
        playlist = next((p for p in PLAYLISTS if p['id'] == value('playlistId')), None)
        if playlist is None:
            playlist = {'id': f'playlist-{len(PLAYLISTS)+1}', 'name': value('name'),
                        'duration': 60, 'public': False, 'owner': 'fixture'}
            PLAYLISTS.append(playlist)
        playlist['songCount'] = len(query.get('songId', []))
        by_id = {song['id']: song for song in SONGS}
        SAVED_PLAYLIST_TRACKS[playlist['id']] = [dict(by_id[song_id]) for song_id in query.get('songId', []) if song_id in by_id]
        return {'playlist': playlist}
    if endpoint == 'updatePlaylist':
        for playlist in PLAYLISTS:
            if playlist['id'] != value('playlistId'): continue
            if value('name'): playlist['name'] = value('name')
            key = playlist['id']
            entries = SAVED_PLAYLIST_TRACKS.get(key, SONGS[30:60] if key == 'playlist-2' else SONGS[:30])
            removed = {int(i) for i in query.get('songIndexToRemove', [])}
            by_id = {song['id']: song for song in SONGS}
            entries = [song for i, song in enumerate(entries) if i not in removed]
            entries.extend(by_id[i] for i in query.get('songIdToAdd', []))
            SAVED_PLAYLIST_TRACKS[key] = entries
            playlist['songCount'] = len(entries)
        return {}
    if endpoint == 'deletePlaylist': PLAYLISTS[:] = [p for p in PLAYLISTS if p['id'] != key]
    return {}

def audio():
    output = io.BytesIO()
    with wave.open(output, 'wb') as stream:
        stream.setnchannels(1); stream.setsampwidth(2); stream.setframerate(8000)
        stream.writeframes(b''.join(struct.pack('<h', int(4000*math.sin(i*math.tau*440/8000))) for i in range(8000*60)))
    return output.getvalue()

def analysis(query):
    selected = query.get('ids', [''])[0].split(',') if query.get('ids') else [s['id'] for s in SONGS]
    limit = int(query.get('limit', ['1000'])[0]); page = int(query.get('page', ['1'])[0])
    selected = selected[(page-1)*limit:page*limit]
    return {'provider_type': 'navidrome', 'has_more': False, 'next_page': None, 'total_tracks': len(selected),
            'tracks': [{'id': key, 'fp': key, 'tempo': 120.0, 'energy': 0.6, 'key': 'C', 'scale': 'major',
                        'mood_vector': '{"happy": 0.8}', 'other_features': '{"rock": 0.7}'} for key in selected]}
