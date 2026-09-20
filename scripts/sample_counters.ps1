# Append CPU, thermal, power, memory, GPU and battery samples until the watched process exits.
# Read-only: it changes no system setting.
param(
  [Parameter(Mandatory = $true)][string]$Csv,
  [Parameter(Mandatory = $true)][int]$WatchPid,
  [int]$IntervalSeconds = 5
)

$counters = @(
  '\Processor(_Total)\% Processor Time',
  '\Processor Information(_Total)\% Processor Performance',
  '\Thermal Zone Information(*)\Temperature',
  '\Thermal Zone Information(*)\% Passive Limit',
  '\Energy Meter(*)\Power',
  '\Memory\Available MBytes',
  '\Memory\Committed Bytes',
  '\GPU Adapter Memory(*)\Shared Usage',
  '\Process(whisper-cli*)\% Processor Time',
  '\Process(llama-server*)\% Processor Time',
  '\Process(explorer*)\% Processor Time',
  '\Process(searchindexer*)\% Processor Time'
)

if (-not (Test-Path $Csv)) { 'timestamp,counter,value' | Out-File -Encoding utf8 $Csv }

while (Get-Process -Id $WatchPid -ErrorAction SilentlyContinue) {
  $rows = New-Object System.Collections.Generic.List[string]
  $sample = Get-Counter -Counter $counters -ErrorAction SilentlyContinue
  if ($sample) {
    foreach ($item in $sample.CounterSamples) {
      $rows.Add(('{0},{1},{2}' -f $sample.Timestamp.ToString('o'),
                 ($item.Path -replace '^\\\\[^\\]+', ''), $item.CookedValue))
    }
  }
  $stamp = (Get-Date).ToString('o')
  foreach ($battery in (Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue)) {
    $rows.Add(('{0},\battery\discharge rate,{1}' -f $stamp, $battery.DischargeRate))
    $rows.Add(('{0},\battery\charge rate,{1}' -f $stamp, $battery.ChargeRate))
    $rows.Add(('{0},\battery\remaining capacity,{1}' -f $stamp, $battery.RemainingCapacity))
    $rows.Add(('{0},\battery\power online,{1}' -f $stamp, [int][bool]$battery.PowerOnline))
  }
  if ($rows.Count -gt 0) { $rows | Out-File -Append -Encoding utf8 $Csv }
  Start-Sleep -Seconds $IntervalSeconds
}
