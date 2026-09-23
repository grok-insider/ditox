export function OptionsView(props: {
  transparency: number;
  previewLines: 1 | 2;
  onTransparency: (percent: number) => void;
  onPreviewLines: (lines: 1 | 2) => void;
  onClose: () => void;
}) {
  return (
    <section className="ditox-options">
      <h1>Options</h1>
      <fieldset>
        <legend>Transparency</legend>
        {[
          [0, "Opaque"],
          [10, "10%"],
          [20, "20%"],
          [30, "30%"],
        ].map(([percent, label]) => (
          <label key={percent}>
            <input
              type="radio"
              name="transparency"
              checked={props.transparency === percent}
              onChange={() => props.onTransparency(Number(percent))}
            />
            {label}
          </label>
        ))}
      </fieldset>
      <fieldset>
        <legend>Preview lines</legend>
        <label>
          <input type="radio" name="lines" checked={props.previewLines === 1} onChange={() => props.onPreviewLines(1)} />1
        </label>
        <label>
          <input type="radio" name="lines" checked={props.previewLines === 2} onChange={() => props.onPreviewLines(2)} />2
        </label>
      </fieldset>
      <button type="button" onClick={props.onClose}>
        OK
      </button>
    </section>
  );
}
