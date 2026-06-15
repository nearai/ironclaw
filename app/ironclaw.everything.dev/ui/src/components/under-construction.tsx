import type { ClientRuntimeConfig } from "everything-dev/types";
import { motion } from "framer-motion";
import { ExternalLink } from "lucide-react";
import { getRepository } from "@/app";
import underConstructionImage from "@/assets/under-construction.gif";
import {
  ClassicTooltipContent,
  Tooltip,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

interface UnderConstructionProps {
  label?: string;
  sourceFile?: string;
  className?: string;
  onClick?: () => void;
  skipNavigation?: boolean;
  pressed?: boolean;
  runtimeConfig?: Partial<ClientRuntimeConfig>;
}

const DEFAULT_REPOSITORY = "https://github.com/nearbuilders/everything-dev";

export function UnderConstruction({
  label,
  sourceFile,
  className,
  onClick,
  skipNavigation,
  pressed,
  runtimeConfig,
}: UnderConstructionProps) {
  const repository = getRepository(runtimeConfig) ?? DEFAULT_REPOSITORY;
  const githubUrl = sourceFile ? `${repository}/blob/main/${sourceFile}` : repository;

  const handleClick = () => {
    onClick?.();
    if (!skipNavigation) {
      setTimeout(() => {
        window.open(githubUrl, "_blank", "noopener,noreferrer");
      }, 150);
    }
  };

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <div className={className} style={{ perspective: 800 }}>
            <button
              type="button"
              className="bg-transparent border-0 transition-transform p-4 -m-4"
              style={{ cursor: "pointer" }}
              onClick={handleClick}
              aria-label={
                skipNavigation
                  ? label
                    ? `${label} under construction`
                    : "under construction"
                  : label
                    ? `${label} under construction - view source`
                    : "under construction - view source"
              }
            >
              <motion.div
                animate={
                  pressed
                    ? { scale: 0.95, rotateY: 0, z: -15 }
                    : {
                        rotateY: [0, 12, 0, -12, 0],
                        y: [0, -4, 0],
                      }
                }
                transition={
                  pressed
                    ? { duration: 0.15 }
                    : {
                        rotateY: {
                          duration: 4,
                          ease: "easeInOut",
                          repeat: Infinity,
                        },
                        y: {
                          duration: 3,
                          ease: "easeInOut",
                          repeat: Infinity,
                        },
                      }
                }
                whileTap={{ scale: 0.95, rotateY: 0, z: -15 }}
                className="relative"
                style={{ transformStyle: "preserve-3d" }}
              >
                <img
                  src={underConstructionImage}
                  alt={label ? `${label} under construction` : "under construction"}
                  className="w-full h-auto rounded-xl border border-border object-cover shadow-lg"
                />
              </motion.div>
            </button>
          </div>
        </TooltipTrigger>
        {!skipNavigation && (
          <ClassicTooltipContent side="top">
            <span className="flex items-center gap-1.5">
              see code and contribute
              <ExternalLink className="w-3 h-3" />
            </span>
          </ClassicTooltipContent>
        )}
      </Tooltip>
    </TooltipProvider>
  );
}
