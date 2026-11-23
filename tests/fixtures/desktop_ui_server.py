"""Loopback OpenSubsonic/WebDAV fixture for native desktop interaction checks.

Uses only deliberately public fixture credentials. Never connects to a music server.
"""
import argparse
import base64
import hashlib
import json
import mimetypes
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse, urlencode
from xml.sax.saxutils import escape
import time

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        if parsed.path.startswith('/slow/'):
            time.sleep(2)
        if self.server.catalog and '/api/' in parsed.path:
            return self.analysis(parsed, query)
        if self.server.catalog and parsed.path == '/fixture-events':
            return self.send_json(self.server.catalog.EVENTS)
        token = hashlib.md5(('fixture-password' + query.get('s', [''])[0]).encode()).hexdigest()
        valid = query.get('u') == ['fixture'] and query.get('t') == [token]
        response = {'status': 'ok', 'version': '1.16.1', 'type': 'navidrome', 'serverVersion': 'fixture'}
        if '/rest/' in parsed.path and not valid:
            response.update(status='failed', error={'code': 40, 'message': 'Fixture authentication failed'})
        else:
            endpoint = parsed.path.rsplit('/', 1)[-1].removesuffix('.view')
            if endpoint == 'getArtists' and self.server.fail_catalog:
                self.send_response(503); self.send_header('Content-Length', '0'); self.end_headers(); return
            if endpoint == 'getArtists' and self.server.scan_delay: time.sleep(self.server.scan_delay)
            if endpoint in ('getPlaylist', 'getArtist', 'getAlbum', 'getArtistInfo2') and self.server.browse_delay: time.sleep(self.server.browse_delay)
            body = {
                'getMusicFolders': {'musicFolders': {'musicFolder': [{'id': 1, 'name': 'Fixture Music'}]}},
                'getOpenSubsonicExtensions': {'openSubsonicExtensions': []},
                'getArtists': {'artists': {'index': []}},
                'getPlaylists': {'playlists': {'playlist': []}},
                'getAlbumList2': {'albumList2': {'album': []}},
                'getGenres': {'genres': {'genre': []}},
                'getRandomSongs': {'randomSongs': {'song': []}},
            }.get(endpoint, {})
            if self.server.catalog:
                if endpoint in ('getCoverArt', 'artist-photo.png'):
                    import struct, zlib
                    def chunk(kind, body):
                        return struct.pack('!I', len(body)) + kind + body + struct.pack('!I', zlib.crc32(kind + body))
                    rgb = hashlib.sha256(query.get('id', ['fixture'])[0].encode()).digest()[:3]
                    rows = b''.join(b'\x00' + rgb * 32 for _ in range(32))
                    data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', 32, 32, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
                    content_type = 'image/png'
                    if self.server.cover_file:
                        data = self.server.cover_file.read_bytes()
                        content_type = mimetypes.guess_type(self.server.cover_file.name)[0] or 'application/octet-stream'
                    self.server.catalog.EVENTS.append({'endpoint': endpoint, 'ids': query.get('id', [])})
                    self.send_response(200); self.send_header('Content-Type', content_type); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
                if endpoint in ('stream', 'download'):
                    self.server.catalog.EVENTS.append({'endpoint': endpoint, 'ids': query.get('id', [])})
                    if endpoint == 'download' and self.server.download_delay: time.sleep(self.server.download_delay)
                    data = self.server.catalog.audio()
                    self.send_response(200); self.send_header('Content-Type', 'audio/wav'); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
                self.server.catalog.EVENTS.append({'endpoint': endpoint, 'ids': query.get('id', query.get('songId', []))})
                body.update(self.server.catalog.response(endpoint, query))
                if self.server.sonic and endpoint == 'findSonicPath':
                    songs = self.server.catalog.SONGS
                    start = next(song for song in songs if song['id'] == query['startSongId'][0])
                    end = next(song for song in songs if song['id'] == query['endSongId'][0])
                    seen = {start['artistId'], end['artistId']}
                    path = [start]
                    for song in songs:
                        if len(path) >= int(query.get('count', ['5'])[0]) - 1: break
                        if song['artistId'] not in seen:
                            seen.add(song['artistId']); path.append(song)
                    path.append(end)
                    body['sonicMatch'] = [{'entry': song, 'similarity': .95} for song in path]
                if endpoint == 'getArtistInfo2':
                    body['artistInfo2']['similarArtist'] = [dict(artist) for artist in self.server.catalog.ARTISTS if artist['id'] != query.get('id', [''])[0]][:5]
                    body['artistInfo2']['largeImageUrl'] = f'http://127.0.0.1:{self.server.server_port}/artist-photo.png' 
            if self.server.sonic and endpoint == 'getOpenSubsonicExtensions':
                body['openSubsonicExtensions'] = [{'name': 'sonicSimilarity', 'versions': [1]}]
            if self.server.sonic and endpoint == 'getSonicSimilarTracks':
                if self.server.sonic_delay: time.sleep(self.server.sonic_delay)
                if self.server.fail_sonic: response.update(status='failed', error={'code': 70, 'message': 'Fixture sonic service unavailable'})
                body['sonicMatch'] = [{'entry': song, 'similarity': .95} for song in self.server.catalog.SONGS[:100]]
            response.update(body)
        data = json.dumps({'subsonic-response': response}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_json(self, value):
        data = json.dumps(value).encode()
        self.send_response(200); self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)

    def do_POST(self):
        parsed = urlparse(self.path)
        data = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        if parsed.path.startswith('/rest/'):
            query = parse_qs(parsed.query)
            query.update(parse_qs(data.decode('utf-8')))
            self.path = parsed.path + '?' + urlencode(query, doseq=True)
            return self.do_GET()
        if parsed.path == '/auth':
            credentials = json.loads(data or b'{}')
            if credentials != {'user': 'fixture', 'password': 'fixture-password'}:
                self.send_response(401); self.send_header('Content-Length', '0'); self.end_headers(); return
            self.send_response(200); self.send_header('Set-Cookie', 'audiomuse_jwt=fixture; HttpOnly'); self.send_header('Content-Length', '2'); self.end_headers(); self.wfile.write(b'{}'); return
        self.analysis(parsed, parse_qs(parsed.query))

    def analysis(self, parsed, query):
        catalog = self.server.catalog
        if not catalog: return self.send_json({})
        path = parsed.path
        if self.server.sonic_delay and ('/search' in path or path.endswith('/similar_tracks') or path.endswith('/find_path')):
            time.sleep(self.server.sonic_delay)
        if path.endswith('/servers'): return self.send_json({'servers': [{'name': 'Navidrome', 'server_type': 'navidrome'}]})
        if path.endswith('/sync'): return self.send_json(catalog.analysis(query))
        if path.endswith('/stats'): return self.send_json({'clap_enabled': True, 'lyrics_enabled': True, 'total_songs': len(catalog.SONGS), 'analyzed_songs': len(catalog.SONGS), 'song_count': len(catalog.SONGS)})
        if path.endswith('/mood_centroids'): return self.send_json({'happy': [{'index': 0}]})
        rows = [{'item_id': song['id']} for song in catalog.SONGS[1:16]]
        return self.send_json({'results': rows} if '/search' in path else rows)

    def do_PROPFIND(self):
        expected = 'Basic ' + base64.b64encode(b'fixture:fixture-password').decode()
        if self.headers.get('Authorization') != expected:
            self.send_response(401)
            self.send_header('WWW-Authenticate', 'Basic realm="Fixture"')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if self.path.startswith('/slow/'):
            time.sleep(2)
        data = (f'<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{escape(self.path)}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>').encode()
        self.send_response(207)
        self.send_header('Content-Type', 'application/xml')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--ready-file', required=True)
    parser.add_argument('--port', type=int, default=0)
    parser.add_argument('--host', default='127.0.0.1')
    parser.add_argument('--fail-catalog', action='store_true')
    parser.add_argument('--catalog', action='store_true')
    parser.add_argument('--artist-count', type=int, default=26)
    parser.add_argument('--sonic', action='store_true')
    parser.add_argument('--fail-sonic', action='store_true')
    parser.add_argument('--sonic-delay', type=float, default=0)
    parser.add_argument('--letter-fixture', action='store_true')
    parser.add_argument('--album-art-only', action='store_true')
    parser.add_argument('--scan-delay', type=float, default=0)
    parser.add_argument('--browse-delay', type=float, default=0)
    parser.add_argument('--download-delay', type=float, default=0)
    parser.add_argument('--cover-file', type=Path)
    args = parser.parse_args()
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.catalog = __import__('desktop_catalog') if args.catalog else None
    if server.catalog and args.artist_count != 26:
        server.catalog.populate(args.artist_count)
    server.fail_catalog = args.fail_catalog
    server.sonic = args.sonic
    server.fail_sonic = args.fail_sonic
    server.sonic_delay = args.sonic_delay
    server.scan_delay = args.scan_delay
    server.browse_delay = args.browse_delay
    server.download_delay = args.download_delay
    server.cover_file = args.cover_file
    if args.album_art_only and server.catalog:
        for song in server.catalog.SONGS: song.pop('coverArt', None)
    if args.letter_fixture and server.catalog:
        for index, name in [(0, 'Air'), (1, 'Alvvays'), (2, 'The Alarm'), (12, 'McCoy'), (13, 'Midlake'), (14, 'Miles Davis')]:
            artist = server.catalog.ARTISTS[index]
            artist['name'] = name
            for row in server.catalog.ALBUMS + server.catalog.SONGS:
                if row.get('artistId') == artist['id']: row['artist'] = name
    Path(args.ready_file).write_text(f'http://{args.host}:{server.server_port}')
    server.serve_forever()
