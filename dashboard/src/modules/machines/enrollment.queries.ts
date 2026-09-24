import { queryOptions } from "@tanstack/react-query";
import { loadOrganizationEnrollmentStatusServerFn } from "./enrollment.functions";

export function organizationEnrollmentStatusQueryOptions(organizationSlug: string) {
  return queryOptions({
    queryKey: ["enrollment-status", organizationSlug],
    // Founder enrollment changes out of band; read it fresh on every visit.
    staleTime: 0,
    queryFn: () => loadOrganizationEnrollmentStatusServerFn({ data: { organizationSlug } }),
  });
}
