import "./index.css";
import { Composition } from "remotion";
import {
  IronClawArchitecture,
  TOTAL_DURATION,
} from "./IronClawArchitecture";

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition
        id="IronClawArchitecture"
        component={IronClawArchitecture}
        durationInFrames={TOTAL_DURATION}
        fps={30}
        width={1280}
        height={720}
      />
    </>
  );
};
