import SwiftUI

struct LyricsScreen: View {
  @Environment(AppModel.self) private var model
  var body: some View {
    if let lyrics = model.snapshot?.lyricsView {
      VStack(spacing: 0) {
        VStack(spacing: 12) {
          Text(lyrics.title).font(.headline).multilineTextAlignment(.center)
          if lyrics.languages.count > 1 {
            Picker(
              "Language",
              selection: Binding(
                get: { lyrics.language },
                set: {
                  model.send("lyrics_language", ["index": $0])
                })
            ) {
              ForEach(lyrics.languages.indices, id: \.self) { index in
                Text(lyrics.languages[index]).tag(index)
              }
            }.pickerStyle(.menu).accessibilityIdentifier("lyrics-language")
          }
          Toggle(
            "Follow playback",
            isOn: Binding(
              get: { lyrics.follow },
              set: { _ in
                model.send("lyrics_follow")
              })
          ).disabled(!lyrics.synced).accessibilityIdentifier("lyrics-follow")
        }.padding()
        ScrollViewReader { proxy in
          ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
              if let message = lyrics.message {
                Text(message).foregroundStyle(model.theme.muted)
                if !lyrics.loading {
                  Button("Reload Lyrics") { model.send("lyrics_retry") }
                  Button("Search Lyrics on the Web") { model.send("lyrics_search") }
                }
              }
              ForEach(lyrics.lines.indices, id: \.self) { index in
                Text(attributed(lyrics.lines[index])).font(.title3)
                  .frame(maxWidth: .infinity, alignment: .leading).id(index)
                  .accessibilityIdentifier(
                    lyrics.lines[index].active ? "lyrics-active-line" : "lyrics-line-\(index)")
              }
            }.padding(24).textSelection(.enabled)
          }
          .onChange(of: lyrics.track) { _, _ in proxy.scrollTo(lyrics.active ?? 0, anchor: .top) }
          .onChange(of: lyrics.language) { _, _ in proxy.scrollTo(lyrics.active ?? 0, anchor: .top)
          }
          .onChange(of: lyrics.active, initial: true) { _, index in
            if lyrics.follow, let index { withAnimation { proxy.scrollTo(index, anchor: .center) } }
          }
        }
      }.background(model.theme.bg).foregroundStyle(model.theme.text)
    } else {
      ProgressView("Loading lyrics…")
    }
  }
  private func attributed(_ line: LyricsDisplayLine) -> AttributedString {
    var result = AttributedString()
    for span in line.spans {
      var part = AttributedString(span.text)
      part.foregroundColor = span.active ? model.theme.tint : model.theme.text
      if span.active {
        part.font = .title3.bold()
      } else if line.active {
        part.font = .title3.weight(.semibold)
      }
      result.append(part)
    }
    return result
  }
}
