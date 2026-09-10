import { useQuery } from "@tanstack/react-query";
import { Alert, AlertDescription } from "#/components/ui/alert";
import { githubReposQueryKey, type GithubCollectionScope } from "#/modules/github/github.collection";

export function GithubRepositoryRefreshNotice({ scope }: { scope: GithubCollectionScope }) {
  // Observe errors even when a failed read leaves all collection rows unchanged.
  const { isError } = useQuery({
    queryKey: githubReposQueryKey(scope),
    enabled: false,
    gcTime: 1,
  }, scope.queryClient);
  if (!isError) return null;
  return (
    <Alert>
      <AlertDescription>
        Could not refresh repositories. Shown results may be out of date. Retrying automatically.
      </AlertDescription>
    </Alert>
  );
}
