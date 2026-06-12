import { BAD_REQUEST, UNAUTHORIZED } from "every-plugin/errors";
import { oc } from "every-plugin/orpc";
import { z } from "every-plugin/zod";

export const RegisterInputSchema = z.object({
  agentId: z.string().min(1).max(64),
  participantName: z.string().min(1).max(128),
  novaAccountId: z.string().min(1).max(128),
});

export const SubmitInputSchema = z.object({
  agentId: z.string().min(1).max(64),
  novaAccountId: z.string().min(1).max(128),
  novaApiKey: z.string().min(1),
  projectTitle: z.string().min(1).max(256),
  description: z.string().min(1).max(280),
  demoUrl: z.string().url(),
  githubUrl: z.string().url().optional(),
  skillsList: z.string().optional(),
  demoNotes: z.string().optional(),
});

export const LeaderboardEntrySchema = z.object({
  agentId: z.string(),
  participantName: z.string(),
  projectTitle: z.string(),
  submittedAt: z.iso.datetime(),
});

export const contract = oc.router({
  ping: oc.route({ method: "GET", path: "/ping" }).output(
    z.object({
      status: z.literal("ok"),
      timestamp: z.iso.datetime(),
    }),
  ),

  hackathon: {
    register: oc
      .route({ method: "POST", path: "/hackathon/register" })
      .input(RegisterInputSchema)
      .output(
        z.object({
          success: z.boolean(),
          message: z.string(),
        }),
      )
      .errors({ UNAUTHORIZED, BAD_REQUEST }),

    submit: oc
      .route({ method: "POST", path: "/hackathon/submit" })
      .input(SubmitInputSchema)
      .output(
        z.object({
          success: z.boolean(),
          cid: z.string(),
          message: z.string(),
        }),
      )
      .errors({ UNAUTHORIZED, BAD_REQUEST }),

    leaderboard: oc.route({ method: "GET", path: "/hackathon/leaderboard" }).output(
      z.object({
        entries: z.array(LeaderboardEntrySchema),
      }),
    ),
  },
});

export type ContractType = typeof contract;
