// Green triangle indicating the Playhead position in the cue list.

interface Props {
  visible: boolean;
}

export function PlayheadIndicator({ visible }: Props) {
  if (!visible) return <span style={{ display: "inline-block", width: 12 }} />;
  return (
    <span
      style={{
        display: "inline-block",
        width: 0,
        height: 0,
        borderTop: "6px solid transparent",
        borderBottom: "6px solid transparent",
        borderLeft: "10px solid #4ade80",
        verticalAlign: "middle",
        flexShrink: 0,
      }}
      title="Playhead"
    />
  );
}
