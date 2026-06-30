import { useQuery } from "@tanstack/react-query";
import { gatewayStatus } from "../lib/api.js";

export function useGatewayStatus(token) {
  return useQuery({
    enabled: Boolean(token),
    queryKey: ["gateway-status", token],
    queryFn: gatewayStatus,
    refetchInterval: 30_000,
  });
}
