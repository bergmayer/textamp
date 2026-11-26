import SwiftUI

struct LyricsScreen: View {
  @Environment(AppModel.self) private var model
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
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
        GeometryReader { viewport in
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
                  Text(attributed(lyrics.lines[index]))
                    .font(lyrics.lines[index].active ? .title2.weight(.semibold) : .title3)
                    .foregroundStyle(lyrics.lines[index].active ? activeColor : model.theme.text)
                    .frame(maxWidth: .infinity, alignment: .leading).id(index)
                    .accessibilityIdentifier(
                      lyrics.lines[index].active ? "lyrics-active-line" : "lyrics-line-\(index)")
                }
              }.padding(24).textSelection(.enabled)
            }
            .accessibilityIdentifier("lyrics-scroll")
            .contentMargins(.vertical, lyrics.synced ? max(0, viewport.size.height / 2 - 34) : 0, for: .scrollContent)
            .onChange(of: ScrollTarget(lyrics, height: viewport.size.height), initial: true) { old, target in
              if let line = target.line {
                withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.22)) {
                  proxy.scrollTo(line, anchor: .center)
                }
              } else if old.track != target.track || old.language != target.language {
                proxy.scrollTo(0, anchor: .top)
              }
            }
          }
        }
      }.background(model.theme.baseFill).foregroundStyle(model.theme.text)
    } else {
      ProgressView("Loading lyrics…")
    }
  }
  private struct ScrollTarget: Equatable {
    let track: String
    let language: Int
    let line: Int?
    let height: CGFloat
    init(_ lyrics: LyricsViewModel, height: CGFloat) {
      self.height = height
      track = lyrics.track
      language = lyrics.language
      line = lyrics.follow ? lyrics.active : nil
    }
  }
  private var activeColor: Color {
    if #available(iOS 18.0, *) { return model.theme.text.mix(with: model.theme.tint, by: 0.4) }
    return model.theme.tint
  }
  private func attributed(_ line: LyricsDisplayLine) -> AttributedString {
    var result = AttributedString()
    for span in line.spans {
      var part = AttributedString(span.text)
      if span.active {
        part.foregroundColor = model.theme.tint
        part.font = .title2.bold()
      }
      result.append(part)
    }
    return result
  }
}
