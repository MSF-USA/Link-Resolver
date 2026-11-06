<#
.SYNOPSIS
Resolves an HTTP(S) redirect URL and prints its final URL.
.DESCRIPTION
Follows up to ten redirects with a thirty-second timeout. Downloads only
response headers. A final HTTP error response still counts as a resolved URL.
Connection failures and redirect loops produce an error and exit code 1.
Requires Windows PowerShell 5.1 or PowerShell 7+.
.PARAMETER Url
The absolute HTTP or HTTPS URL to resolve.
.EXAMPLE
./resolve-url.ps1 -Url 'https://example.com/redirect'
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string] $Url
)

$ErrorActionPreference = 'Stop'
$handler = $null
$client = $null
$response = $null

try {
    $parsedUrl = $null
    if (-not [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref] $parsedUrl) -or
        $parsedUrl.Scheme -notin @('http', 'https')) {
        throw 'Provide an absolute HTTP or HTTPS URL: ./resolve-url.ps1 -Url <url>'
    }

    Add-Type -AssemblyName System.Net.Http
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.MaxAutomaticRedirections = 10
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(30)

    # GET works with redirect services that do not support HEAD requests.
    $response = $client.GetAsync(
        $parsedUrl,
        [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead
    ).GetAwaiter().GetResult()

    # Some .NET versions return the last redirect instead of throwing at the limit.
    if ([int] $response.StatusCode -in @(301, 302, 303, 307, 308) -and
        $null -ne $response.Headers.Location) {
        throw 'Redirect was not followed: the limit was reached or the redirect was disallowed.'
    }

    Write-Output $response.RequestMessage.RequestUri.AbsoluteUri
}
catch {
    Write-Error "Could not resolve URL: $($_.Exception.Message)" -ErrorAction Continue
    exit 1
}
finally {
    if ($null -ne $response) { $response.Dispose() }
    if ($null -ne $client) { $client.Dispose() }
    elseif ($null -ne $handler) { $handler.Dispose() }
}
