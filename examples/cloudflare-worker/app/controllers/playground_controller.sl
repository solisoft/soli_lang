const PLAYGROUND_SAMPLE = "Soli runs on the edge. The edge is close to your users, and Soli is fast, fast, fast."

class PlaygroundController < Controller
  # GET /try — word statistics, computed by Soli inside the Worker
  def show(req)
    @title = "Try it"
    @runtime = RuntimeInfo.of(req)
    @text = params["text"].to_s.trim
    @text = PLAYGROUND_SAMPLE if @text.blank?
    @stats = @_stats(@text)
  end

  private

  def _stats(text)
    words = text.downcase.split(" ")
      .map { |w| w.gsub("[^a-z0-9'àâäéèêëîïôöùûüç-]", "") }
      .filter { |w| w.length > 0 }
    counts = {}
    words.each do |w|
      counts[w] = (counts[w] ?? 0) + 1
    end
    top = counts.keys.sort_by { |w| -counts[w] }.take(5).map { |w| {"word": w, "count": counts[w]} }
    {
      "characters": text.chars.length,
      "words": words.length,
      "unique": counts.keys.length,
      "longest": words.sort_by { |w| -w.chars.length }.first.to_s,
      "reading_seconds": (words.length * 60 / 230.0).round(1),
      "top": top
    }
  end
end
