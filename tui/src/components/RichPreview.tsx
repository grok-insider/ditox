import { createEffect, Show } from "solid-js";
import type { ScrollBoxRenderable } from "@opentui/core";
import { classifyPreview, codeFiletype, previewSyntaxStyle } from "../preview-syntax";
import type { TuiTheme } from "../theme";

const linearScroll = { tick: () => 1, reset: () => {} };

export function RichPreview(props: { content: string; height: number; fg: string; bg: string; scrollTop: number; theme: TuiTheme }) {
  const kind = () => classifyPreview(props.content);
  let scroller: ScrollBoxRenderable | undefined;
  createEffect(() => {
    const top = props.scrollTop;
    if (scroller) scroller.scrollTop = top;
  });
  return (
    <Show when={kind() !== "plain"}>
      <scrollbox
        ref={(element) => {
          scroller = element as ScrollBoxRenderable;
          scroller.scrollTop = props.scrollTop;
        }}
        width="100%"
        height={Math.max(1, props.height)}
        overflow="hidden"
        stickyScroll={false}
        scrollAcceleration={linearScroll}
        verticalScrollbarOptions={{ visible: false }}
        horizontalScrollbarOptions={{ visible: false }}
      >
        <Show
          when={kind() === "markdown"}
          fallback={
            <code
              width="100%"
              content={props.content}
              filetype={codeFiletype(props.content)}
              syntaxStyle={previewSyntaxStyle(props.theme)}
              fg={props.fg}
              bg={props.bg}
              drawUnstyledText
            />
          }
        >
          <markdown width="100%" content={props.content} syntaxStyle={previewSyntaxStyle(props.theme)} fg={props.fg} bg={props.bg} conceal />
        </Show>
      </scrollbox>
    </Show>
  );
}
