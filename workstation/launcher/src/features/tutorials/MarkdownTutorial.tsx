import { ImageSquare } from "@phosphor-icons/react/ImageSquare";
import { SpinnerGap } from "@phosphor-icons/react/SpinnerGap";
import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import type { EngineId } from "../../navigation";
import { readTutorialAsset } from "../../workstationApi";

type MarkdownTutorialProps = {
  engine: EngineId;
  markdown: string;
};

export function MarkdownTutorial({ engine, markdown }: MarkdownTutorialProps) {
  return (
    <div className="tutorial-markdown">
      <ReactMarkdown
        components={{
          a({ children, href, node: _node, ...props }) {
            const external = Boolean(href && /^https?:\/\//i.test(href));
            return (
              <a
                {...props}
                href={href}
                rel={external ? "noreferrer noopener" : undefined}
                target={external ? "_blank" : undefined}
              >
                {children}
              </a>
            );
          },
          img({ alt, src, title }) {
            return (
              <TutorialImage
                alt={alt ?? "教程图片"}
                engine={engine}
                source={src}
                title={title}
              />
            );
          },
          table({ children, node: _node, ...props }) {
            return (
              <div className="tutorial-table-scroll">
                <table {...props}>{children}</table>
              </div>
            );
          },
        }}
        remarkPlugins={[remarkGfm]}
      >
        {markdown}
      </ReactMarkdown>
    </div>
  );
}

function TutorialImage({
  alt,
  engine,
  source,
  title,
}: {
  alt: string;
  engine: EngineId;
  source?: string;
  title?: string;
}) {
  const [resolvedSource, setResolvedSource] = useState<string | null>(() =>
    isDirectImageSource(source) ? source ?? null : null,
  );
  const [loading, setLoading] = useState(
    Boolean(source && !isDirectImageSource(source)),
  );
  const [failed, setFailed] = useState(!source);

  useEffect(() => {
    let cancelled = false;
    if (!source) {
      setResolvedSource(null);
      setLoading(false);
      setFailed(true);
      return () => {
        cancelled = true;
      };
    }
    if (isDirectImageSource(source)) {
      setResolvedSource(source);
      setLoading(false);
      setFailed(false);
      return () => {
        cancelled = true;
      };
    }

    setResolvedSource(null);
    setLoading(true);
    setFailed(false);
    void readTutorialAsset(engine, source)
      .then((asset) => {
        if (!cancelled) setResolvedSource(asset.dataUrl);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [engine, source]);

  if (loading) {
    return (
      <span className="tutorial-image-state" role="status">
        <SpinnerGap className="spin" aria-hidden="true" />
        正在读取教程图片
      </span>
    );
  }
  if (failed || !resolvedSource) {
    return (
      <span className="tutorial-image-state tutorial-image-state--error">
        <ImageSquare aria-hidden="true" />
        图片资源暂未找到：{alt}
      </span>
    );
  }
  return (
    <img
      alt={alt}
      decoding="async"
      loading="lazy"
      onError={() => setFailed(true)}
      referrerPolicy="no-referrer"
      src={resolvedSource}
      title={title}
    />
  );
}

function isDirectImageSource(source?: string): boolean {
  return Boolean(source && /^(?:https?:|data:)/i.test(source));
}
